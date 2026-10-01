//! One reading of the machine: plain data, cheap to clone and send to the UI.

use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at: SystemTime,
    pub cpu: Cpu,
    pub memory: Memory,
    /// Every GPU we can read, busiest first.
    pub gpus: Vec<Gpu>,
    /// Why `gpus` is empty, for the hover card.
    pub gpu_note: Option<String>,
    pub network: Network,
    /// The processes worth naming: the top few by CPU and by memory, and every one holding GPU
    /// memory. Not the whole table, so a reading stays small.
    pub processes: Vec<Process>,
    /// Totals for each tracked root process and everything it started (see `Monitor::set_roots`).
    pub trees: Vec<Tree>,
}

impl Sample {
    /// The busiest GPU, which is what the gauge shows.
    pub fn gpu(&self) -> Option<&Gpu> {
        self.gpus.first()
    }

    pub fn tree(&self, root: u32) -> Option<&Tree> {
        self.trees.iter().find(|t| t.root == root)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cpu {
    /// Percent of the whole machine (every core), 0–100.
    pub usage: f32,
    /// Logical cores.
    pub cores: usize,
    pub brand: String,
    /// 1, 5 and 15-minute load averages (Linux and macOS).
    pub load: Option<[f64; 3]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Memory {
    /// Bytes in use: total minus what the OS could hand out without swapping.
    pub used: u64,
    pub total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
}

impl Memory {
    pub fn percent(&self) -> f32 {
        percent(self.used, self.total)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Gpu {
    pub name: String,
    /// Percent of time the GPU was busy.
    pub usage: Option<f32>,
    pub memory_used: Option<u64>,
    /// `None` on unified-memory machines, where GPU memory is system memory.
    pub memory_total: Option<u64>,
    /// Degrees Celsius.
    pub temperature: Option<u32>,
    pub power_watts: Option<f32>,
}

impl Gpu {
    pub fn memory_percent(&self) -> Option<f32> {
        Some(percent(self.memory_used?, self.memory_total?))
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    /// Bytes per second received across the physical interfaces.
    pub rx: u64,
    /// Bytes per second sent across the physical interfaces.
    pub tx: u64,
    /// Physical interfaces (no loopback, containers or VPN tunnels, whose traffic is already
    /// counted on a real one), busiest first.
    pub interfaces: Vec<Interface>,
}

impl Network {
    /// How full the busiest link is in its busier direction (links are full duplex), when the
    /// link reports its speed.
    pub fn utilization(&self) -> Option<f32> {
        let busiest = self.interfaces.first()?;
        let link = busiest.link?;
        Some(percent(busiest.rx.max(busiest.tx), link))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    pub rx: u64,
    pub tx: u64,
    /// Link capacity in bytes per second, when the driver reports it.
    pub link: Option<u64>,
}

impl Interface {
    pub fn total(&self) -> u64 {
        self.rx + self.tx
    }
}

/// A program: every process with the same name under the same root, totalled.
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    /// The group's lowest pid (usually the one that started the others).
    pub pid: u32,
    /// A readable name: `claude-code` for `node …/claude-code/cli.js`, not `node`.
    pub name: String,
    /// How many processes the entry stands for.
    pub count: u32,
    /// Percent of the whole machine, like [`Cpu::usage`] (not of one core).
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
    /// GPU memory in bytes, when a GPU reports it per process.
    pub gpu_memory: Option<u64>,
    /// The tracked root this process descends from.
    pub root: Option<u32>,
}

/// A tracked process and all its descendants, totalled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tree {
    pub root: u32,
    /// Percent of the whole machine.
    pub cpu: f32,
    pub memory: u64,
    pub gpu_memory: u64,
    /// How many processes, the root included.
    pub processes: u32,
}

pub(crate) fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 { 0.0 } else { (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0) as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utilization_uses_the_busier_direction_of_the_busiest_link() {
        let mut net = Network::default();
        assert_eq!(net.utilization(), None);
        net.interfaces.push(Interface { name: "eth0".into(), rx: 25, tx: 5, link: Some(100) });
        assert_eq!(net.utilization(), Some(25.0));
        net.interfaces[0].link = None;
        assert_eq!(net.utilization(), None);
    }

    #[test]
    fn percentages_are_clamped() {
        assert_eq!(percent(5, 0), 0.0);
        assert_eq!(percent(150, 100), 100.0);
        let m = Memory { used: 16, total: 64, ..Default::default() };
        assert_eq!(m.percent(), 25.0);
    }
}
