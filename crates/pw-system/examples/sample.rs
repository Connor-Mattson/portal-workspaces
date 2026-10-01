//! Prints a few readings and how long each took:
//! `cargo run -p pw-system --release --example sample -- [root pid…]`

use std::time::{Duration, Instant};

use pw_system::{Sampler, Source};

fn main() {
    let roots: Vec<u32> = std::env::args().skip(1).filter_map(|a| a.parse().ok()).collect();
    let started = Instant::now();
    let mut sampler = Sampler::new();
    println!("probe: {:?}", started.elapsed());
    for _ in 0..3 {
        std::thread::sleep(Duration::from_secs(2));
        let started = Instant::now();
        let sample = sampler.sample(&roots);
        println!("sample: {:?}\n{sample:#?}\n", started.elapsed());
    }
}
