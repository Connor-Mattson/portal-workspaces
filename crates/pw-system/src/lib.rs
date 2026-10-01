//! System profile for Portal Workspaces: a light, periodic look at CPU, GPU, memory and
//! network, and at which processes are using them.
//!
//! Readings are snapshots every few seconds, not a live trace, and only while the app is showing
//! them (see [`Monitor`]). Processes can be totalled per tracked root (the app's terminals), so
//! the app can say which workspace is busy.
//!
//! Like `pw-usage`, this crate has no GUI types: the app gets plain [`Sample`]s through a
//! callback.

mod gpu;
mod monitor;
mod processes;
mod sample;
mod sampler;

pub use monitor::{INTERVAL, Monitor, WARMUP};
pub use sample::{Cpu, Gpu, Interface, Memory, Network, Process, Sample, Tree};
pub use sampler::Sampler;

/// Where readings come from. The real one is [`Sampler`]; tests substitute their own.
pub trait Source: 'static {
    /// Reads the counters without reporting, so the next [`Source::sample`]'s rates start now.
    fn prime(&mut self) {}

    /// Takes a reading. Processes descending from `roots` are totalled into [`Sample::trees`].
    fn sample(&mut self, roots: &[u32]) -> Sample;
}
