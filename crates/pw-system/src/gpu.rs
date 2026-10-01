//! GPU readings, from whatever the platform offers without root:
//!
//! - **NVIDIA (Linux):** NVML, the library behind `nvidia-smi`, loaded at run time. Machines
//!   without the driver just don't find it. NVML also says which processes hold GPU memory.
//! - **AMD (Linux):** the amdgpu driver's sysfs files (`gpu_busy_percent`, `mem_info_vram_*`).
//! - **Apple:** the `PerformanceStatistics` the GPU driver publishes in the I/O Registry, read
//!   with `ioreg` (a fixed command, a few milliseconds).
//!
//! Intel GPUs on Linux only report utilization through perf counters that need root, so they
//! aren't shown.

use std::collections::HashMap;

use crate::sample::Gpu;

pub(crate) struct Gpus {
    backends: Vec<Backend>,
}

enum Backend {
    #[cfg(target_os = "linux")]
    // NVML's function table is ~12 KB; boxed so the other backends don't carry it.
    Nvidia(Box<nvml_wrapper::Nvml>),
    #[cfg(target_os = "linux")]
    Amd(Vec<std::path::PathBuf>),
    #[cfg(target_os = "macos")]
    Apple,
}

/// GPUs, plus the GPU memory each process holds (by pid, in bytes).
pub(crate) type Reading = (Vec<Gpu>, HashMap<u32, u64>);

impl Gpus {
    /// Looks for every source once. NVML's library load is the slow part (tens of ms), so this
    /// runs on the monitor thread.
    pub(crate) fn probe() -> Self {
        let mut backends = Vec::new();
        #[cfg(target_os = "linux")]
        {
            match nvml_wrapper::Nvml::init() {
                Ok(nvml) if nvml.device_count().is_ok_and(|n| n > 0) => backends.push(Backend::Nvidia(Box::new(nvml))),
                Ok(_) => {}
                Err(err) => tracing::debug!(%err, "no NVML"),
            }
            let cards = amd::cards(std::path::Path::new("/sys/class/drm"));
            if !cards.is_empty() {
                backends.push(Backend::Amd(cards));
            }
        }
        #[cfg(target_os = "macos")]
        backends.push(Backend::Apple);
        Self { backends }
    }

    /// Why there's nothing to show, when there isn't.
    pub(crate) fn note(&self) -> Option<&'static str> {
        if !self.backends.is_empty() {
            return None;
        }
        Some(if cfg!(target_os = "linux") {
            "No NVIDIA or AMD GPU found. Intel GPUs don't report their load without root."
        } else {
            "GPU load isn't available on this system."
        })
    }

    pub(crate) fn read(&self) -> Reading {
        let mut gpus = Vec::new();
        let mut processes: HashMap<u32, u64> = HashMap::new();
        for backend in &self.backends {
            match backend {
                #[cfg(target_os = "linux")]
                Backend::Nvidia(nvml) => nvidia::read(nvml, &mut gpus, &mut processes),
                #[cfg(target_os = "linux")]
                Backend::Amd(cards) => gpus.extend(cards.iter().filter_map(|c| amd::read(c))),
                #[cfg(target_os = "macos")]
                Backend::Apple => gpus.extend(apple::read()),
            }
        }
        gpus.sort_by(|a, b| b.usage.unwrap_or(0.0).total_cmp(&a.usage.unwrap_or(0.0)));
        (gpus, processes)
    }
}

#[cfg(target_os = "linux")]
mod nvidia {
    use std::collections::HashMap;

    use nvml_wrapper::Nvml;
    use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
    use nvml_wrapper::enums::device::UsedGpuMemory;

    use crate::sample::Gpu;

    pub fn read(nvml: &Nvml, gpus: &mut Vec<Gpu>, processes: &mut HashMap<u32, u64>) {
        let count = nvml.device_count().unwrap_or(0);
        for device in (0..count).filter_map(|i| nvml.device_by_index(i).ok()) {
            let memory = device.memory_info().ok();
            gpus.push(Gpu {
                name: device.name().unwrap_or_else(|_| "NVIDIA GPU".into()),
                usage: device.utilization_rates().ok().map(|u| u.gpu as f32),
                memory_used: memory.as_ref().map(|m| m.used),
                memory_total: memory.as_ref().map(|m| m.total),
                temperature: device.temperature(TemperatureSensor::Gpu).ok(),
                power_watts: device.power_usage().ok().map(|mw| mw as f32 / 1000.0),
            });
            // A process doing both compute and graphics appears in both lists with the same
            // allocation; count it once per GPU.
            let mut on_device: HashMap<u32, u64> = HashMap::new();
            let lists = [device.running_compute_processes(), device.running_graphics_processes()];
            for info in lists.into_iter().flatten().flatten() {
                if let UsedGpuMemory::Used(bytes) = info.used_gpu_memory {
                    let slot = on_device.entry(info.pid).or_default();
                    *slot = (*slot).max(bytes);
                }
            }
            for (pid, bytes) in on_device {
                *processes.entry(pid).or_default() += bytes;
            }
        }
    }
}

#[cfg(any(target_os = "linux", test))]
mod amd {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::sample::Gpu;

    /// The `device` folders of cards whose driver reports a busy percentage (amdgpu does).
    pub fn cards(drm: &Path) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(drm) else { return Vec::new() };
        let mut cards: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                // `card0`, not the connectors (`card0-DP-1`).
                name.strip_prefix("card").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
            })
            .map(|e| e.path().join("device"))
            .filter(|d| d.join("gpu_busy_percent").exists())
            .collect();
        cards.sort();
        cards
    }

    pub fn read(device: &Path) -> Option<Gpu> {
        let number = |file: &str| fs::read_to_string(device.join(file)).ok()?.trim().parse::<u64>().ok();
        let usage = number("gpu_busy_percent")? as f32;
        let hwmon = fs::read_dir(device.join("hwmon")).ok().and_then(|mut d| d.next()?.ok()).map(|e| e.path());
        let sensor = |file: &str| {
            let path = hwmon.as_ref()?.join(file);
            fs::read_to_string(path).ok()?.trim().parse::<u64>().ok()
        };
        let name = fs::read_to_string(device.join("product_name"))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "AMD GPU".into());
        Some(Gpu {
            name,
            usage: Some(usage),
            memory_used: number("mem_info_vram_used"),
            memory_total: number("mem_info_vram_total"),
            temperature: sensor("temp1_input").map(|milli| (milli / 1000) as u32),
            power_watts: sensor("power1_average").or_else(|| sensor("power1_input")).map(|uw| uw as f32 / 1e6),
        })
    }
}

#[cfg(any(target_os = "macos", test))]
mod apple {
    use crate::sample::Gpu;

    #[cfg(target_os = "macos")]
    pub fn read() -> Vec<Gpu> {
        let output = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"])
            .stderr(std::process::Stdio::null())
            .output();
        match output {
            Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
            _ => Vec::new(),
        }
    }

    /// One GPU per `IOAccelerator` entry that publishes `PerformanceStatistics`.
    pub fn parse(ioreg: &str) -> Vec<Gpu> {
        ioreg
            .split("+-o ")
            .filter_map(|entry| {
                let stats = entry.lines().find(|l| l.contains("\"PerformanceStatistics\""))?;
                let usage = number(stats, "Device Utilization %")?;
                let model = entry
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("\"model\" = \""))
                    .and_then(|rest| rest.split('"').next())
                    .map(str::to_owned);
                // Discrete GPUs (Intel Macs) report VRAM; Apple silicon shares system memory.
                let vram_used = number(stats, "vramUsedBytes");
                let vram_free = number(stats, "vramFreeBytes");
                let memory_used = vram_used.or_else(|| number(stats, "In use system memory"));
                Some(Gpu {
                    name: model.unwrap_or_else(|| "GPU".into()),
                    usage: Some(usage.min(100) as f32),
                    memory_used,
                    memory_total: vram_used.zip(vram_free).map(|(u, f)| u + f),
                    temperature: None,
                    power_watts: None,
                })
            })
            .collect()
    }

    /// `"key"=123` inside an I/O Registry dictionary.
    fn number(dict: &str, key: &str) -> Option<u64> {
        let at = dict.find(&format!("\"{key}\"="))? + key.len() + 3;
        let digits: String = dict[at..].chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn amd_cards_are_read_from_sysfs() {
        let drm = tempfile::tempdir().unwrap();
        let device = drm.path().join("card1/device");
        fs::create_dir_all(device.join("hwmon/hwmon3")).unwrap();
        fs::create_dir_all(drm.path().join("card1-DP-1")).unwrap();
        fs::create_dir_all(drm.path().join("card0/device")).unwrap(); // no busy file: not amdgpu
        fs::write(device.join("gpu_busy_percent"), "37\n").unwrap();
        fs::write(device.join("mem_info_vram_used"), "1073741824\n").unwrap();
        fs::write(device.join("mem_info_vram_total"), "8589934592\n").unwrap();
        fs::write(device.join("hwmon/hwmon3/temp1_input"), "54000\n").unwrap();
        fs::write(device.join("hwmon/hwmon3/power1_average"), "45000000\n").unwrap();

        let cards = amd::cards(drm.path());
        assert_eq!(cards, vec![device.clone()]);
        let gpu = amd::read(&device).unwrap();
        assert_eq!(gpu.name, "AMD GPU");
        assert_eq!(gpu.usage, Some(37.0));
        assert_eq!(gpu.memory_percent(), Some(12.5));
        assert_eq!(gpu.temperature, Some(54));
        assert_eq!(gpu.power_watts, Some(45.0));
    }

    #[test]
    fn apple_statistics_are_parsed() {
        let ioreg = r#"+-o AGXAcceleratorG13X  <class AGXAcceleratorG13X, id 0x100000a2c, registered, matched, active, busy 0 (0 ms), retain 98>
    {
      "IOClass" = "AGXAcceleratorG13X"
      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=3215441920,"Tiler Utilization %"=4,"recoveryCount"=0,"Renderer Utilization %"=11,"Device Utilization %"=12,"In use system memory"=812171264}
      "model" = "Apple M1 Pro"
    }
"#;
        let gpus = apple::parse(ioreg);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Apple M1 Pro");
        assert_eq!(gpus[0].usage, Some(12.0));
        assert_eq!(gpus[0].memory_used, Some(812_171_264));
        assert_eq!(gpus[0].memory_total, None);
        assert!(apple::parse("+-o IOAcceleratorES  <class …>\n    {\n    }\n").is_empty());
    }
}
