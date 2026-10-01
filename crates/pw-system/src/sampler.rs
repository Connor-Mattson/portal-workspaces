//! The real [`Source`]: sysinfo for CPU, memory, processes and network, plus [`Gpus`].
//!
//! A reading refreshes only what it shows: global CPU, memory, each process's CPU time, memory
//! and parent (its command line once, when first seen), interface counters, and the GPUs. No
//! disks, users, environments or threads.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::Source;
use crate::gpu::Gpus;
use crate::processes::{self, Row};
use crate::sample::{Cpu, Interface, Memory, Network, Sample};

pub struct Sampler {
    system: System,
    networks: Networks,
    gpus: Gpus,
    /// Whether each interface is a real one, decided once per name.
    physical: HashMap<String, bool>,
    /// When the counters were last read, so totals become rates.
    refreshed: Instant,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_cpu_list(CpuRefreshKind::nothing().with_cpu_usage());
        let mut sampler = Self {
            system,
            networks: Networks::new_with_refreshed_list(),
            gpus: Gpus::probe(),
            physical: HashMap::new(),
            refreshed: Instant::now(),
        };
        sampler.refresh();
        sampler
    }

    /// Reads every counter, returning how long it's been since the last read.
    fn refresh(&mut self) -> Duration {
        self.system.refresh_cpu_specifics(CpuRefreshKind::nothing().with_cpu_usage());
        self.system.refresh_memory_specifics(MemoryRefreshKind::everything());
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            // `nothing()` still lists every thread as a process; they would be counted twice.
            ProcessRefreshKind::nothing().without_tasks().with_cpu().with_memory().with_cmd(UpdateKind::OnlyIfNotSet),
        );
        self.networks.refresh(true);
        let now = Instant::now();
        let elapsed = now.duration_since(self.refreshed);
        self.refreshed = now;
        elapsed
    }

    fn cpu(&self) -> Cpu {
        let load = System::load_average();
        let cpus = self.system.cpus();
        Cpu {
            usage: self.system.global_cpu_usage().clamp(0.0, 100.0),
            cores: cpus.len().max(1),
            brand: cpus.first().map(|c| tidy_brand(c.brand())).unwrap_or_default(),
            load: (cfg!(unix) && load.one + load.five + load.fifteen > 0.0).then_some([
                load.one,
                load.five,
                load.fifteen,
            ]),
        }
    }

    fn memory(&self) -> Memory {
        let s = &self.system;
        Memory {
            used: s.total_memory().saturating_sub(s.available_memory()),
            total: s.total_memory(),
            swap_used: s.used_swap(),
            swap_total: s.total_swap(),
        }
    }

    fn network(&mut self, elapsed: Duration) -> Network {
        let secs = elapsed.as_secs_f64().max(0.001);
        let rate = |bytes: u64| (bytes as f64 / secs).round() as u64;
        let mut interfaces = Vec::new();
        for (name, data) in &self.networks {
            let physical = *self.physical.entry(name.clone()).or_insert_with(|| is_physical(name));
            if physical {
                interfaces.push(Interface {
                    name: name.clone(),
                    rx: rate(data.received()),
                    tx: rate(data.transmitted()),
                    link: link_speed(name),
                });
            }
        }
        interfaces.sort_by(|a, b| b.total().cmp(&a.total()).then_with(|| a.name.cmp(&b.name)));
        Network { rx: interfaces.iter().map(|i| i.rx).sum(), tx: interfaces.iter().map(|i| i.tx).sum(), interfaces }
    }

    fn rows(&self, gpu_memory: &HashMap<u32, u64>) -> Vec<Row> {
        let cores = self.system.cpus().len().max(1) as f32;
        self.system
            .processes()
            .values()
            .map(|p| {
                let pid = p.pid().as_u32();
                let cmd: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
                Row {
                    pid,
                    parent: p.parent().map(|pp| pp.as_u32()),
                    name: processes::display_name(&p.name().to_string_lossy(), &cmd),
                    // sysinfo counts a busy core as 100%; the gauge counts the whole machine.
                    cpu: p.cpu_usage() / cores,
                    memory: p.memory(),
                    gpu_memory: gpu_memory.get(&pid).copied(),
                }
            })
            .collect()
    }
}

impl Source for Sampler {
    fn prime(&mut self) {
        self.refresh();
    }

    fn sample(&mut self, roots: &[u32]) -> Sample {
        let elapsed = self.refresh();
        let (gpus, gpu_memory) = self.gpus.read();
        let rows = self.rows(&gpu_memory);
        let owners = processes::attribute(&rows, roots);
        Sample {
            at: SystemTime::now(),
            cpu: self.cpu(),
            memory: self.memory(),
            gpu_note: gpus.is_empty().then(|| self.gpus.note().unwrap_or("No GPU reported its load.").to_owned()),
            gpus,
            network: self.network(elapsed),
            processes: processes::notable(&rows, &owners),
            trees: processes::trees(&rows, &owners, roots),
        }
    }
}

/// The CPU's name without the marketing: "12th Gen Intel(R) Core(TM) i9-12900KF" is
/// "Intel Core i9-12900KF", and "AMD Ryzen 9 5950X 16-Core Processor" is "AMD Ryzen 9 5950X".
fn tidy_brand(brand: &str) -> String {
    let brand = brand.split(" @ ").next().unwrap_or(brand);
    let mut words: Vec<&str> = Vec::new();
    for word in brand.split_whitespace() {
        let word = word.trim_end_matches("(R)").trim_end_matches("(TM)").trim_end_matches(['®', '™']);
        let generation = word.ends_with("th") && word[..word.len() - 2].chars().all(|c| c.is_ascii_digit());
        if word.is_empty() || (words.is_empty() && generation) || (word == "Gen" && words.is_empty()) {
            continue;
        }
        words.push(word);
    }
    while let Some(last) = words.last() {
        if ["CPU", "Processor"].contains(last) || last.ends_with("-Core") {
            words.pop();
        } else {
            break;
        }
    }
    words.join(" ")
}

/// Real network hardware, not loopback, bridges, containers' veths or VPN tunnels (their
/// traffic also crosses a real interface, so counting them would count it twice).
#[cfg(target_os = "linux")]
fn is_physical(name: &str) -> bool {
    std::path::Path::new("/sys/class/net").join(name).join("device").exists()
}

#[cfg(target_os = "macos")]
fn is_physical(name: &str) -> bool {
    // Ethernet and Wi-Fi are `en*`; `lo0`, `utun*`, `awdl0`, `llw0`, `bridge*` are virtual.
    name.starts_with("en")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn is_physical(name: &str) -> bool {
    !name.starts_with("lo")
}

/// Link capacity in bytes per second. Wi-Fi and down links don't say.
#[cfg(target_os = "linux")]
fn link_speed(name: &str) -> Option<u64> {
    let path = std::path::Path::new("/sys/class/net").join(name).join("speed");
    let mbps: i64 = std::fs::read_to_string(path).ok()?.trim().parse().ok()?;
    (mbps > 0).then(|| mbps as u64 * 1_000_000 / 8)
}

#[cfg(not(target_os = "linux"))]
fn link_speed(_: &str) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brands_lose_the_marketing() {
        assert_eq!(tidy_brand("12th Gen Intel(R) Core(TM) i9-12900KF"), "Intel Core i9-12900KF");
        assert_eq!(tidy_brand("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz"), "Intel Core i7-8700");
        assert_eq!(tidy_brand("AMD Ryzen 9 5950X 16-Core Processor   "), "AMD Ryzen 9 5950X");
        assert_eq!(tidy_brand("Apple M1 Pro"), "Apple M1 Pro");
        assert_eq!(tidy_brand(""), "");
    }
}
