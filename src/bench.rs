//! What does 139,255 neurons at 1 kHz actually cost on this machine?
//!
//! Every number downstream of here — the step size, whether to parallelise,
//! whether to prune the connectome — depends on the answer, and the answer was
//! only ever an estimate until this ran. So it runs first, on the real
//! circuit, before any of the graphics exists.

use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

use crate::circuit::Circuit;
use crate::lif::{Params, Sim};

/// Peak resident set size in MB, from the kernel rather than estimated.
fn peak_rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            let kb: f64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
            return Some(kb / 1024.0);
        }
    }
    None
}

struct Row {
    label: String,
    edges: usize,
    synth_seconds: f64,
    wall_seconds: f64,
    mean_hz: f32,
    active_pct: f32,
    total_spikes: u64,
}

impl Row {
    fn realtime(&self) -> f64 {
        self.synth_seconds / self.wall_seconds
    }
}

fn measure(c: &Circuit, params: Params, seconds: f64, label: &str, seeds: u64) -> Row {
    let ticks = (seconds * 1000.0) as u64;
    let mut s = Sim::new(c, params, 0x_F1_1E);
    let t0 = Instant::now();
    for _ in 0..ticks {
        s.step(c);
    }
    let wall = t0.elapsed().as_secs_f64();
    let v = s.vitals();
    let _ = seeds;
    Row {
        label: label.to_string(),
        edges: c.n_edges(),
        synth_seconds: seconds,
        wall_seconds: wall,
        mean_hz: v.mean_hz,
        active_pct: 100.0 * v.active as f32 / c.n() as f32,
        total_spikes: v.total_spikes,
    }
}

pub fn run(data_dir: &Path, seconds: f64) -> Result<()> {
    let path = data_dir.join("brain.bin");
    let t0 = Instant::now();
    let c = Circuit::load(&path)?;
    c.validate()?;
    println!(
        "loaded {} neurons / {} edges in {:.2} s",
        c.n(),
        c.n_edges(),
        t0.elapsed().as_secs_f64()
    );
    if let Some(mb) = peak_rss_mb() {
        println!("peak RSS after load     {mb:.0} MB");
    }
    println!();

    // A short warm-up so the first measurement is not paying for page faults
    // and a cold branch predictor.
    {
        let mut w = Sim::new(&c, Params::default(), 1);
        for _ in 0..200 {
            w.step(&c);
        }
    }

    let mut rows = Vec::new();

    // 1. The circuit as it comes out of the ETL.
    eprintln!("running full circuit for {seconds} simulated seconds...");
    let full = measure(&c, Params::default(), seconds, "full connectome", 0);
    // Sanity: a network that is entirely silent or entirely on fire is not a
    // result, it is a mis-set operating point, and it invalidates the timing.
    print_vitals_warning(&full);
    rows.push(full);

    // 2. Pruned variants — the first fallback lever if the full circuit is too
    //    slow to run in real time.
    for &min_abs in &[2.0f32, 5.0] {
        let p = c.pruned(min_abs);
        let label = format!("drop |w| < {min_abs:.0} synapses");
        eprintln!("running {label} ({} edges)...", p.n_edges());
        rows.push(measure(&p, Params::default(), seconds, &label, 0));
    }

    println!();
    println!(
        "{:<28} {:>10} {:>9} {:>9} {:>10} {:>9} {:>9}",
        "config", "edges", "sim s", "wall s", "×realtime", "mean Hz", "active%"
    );
    println!("{}", "-".repeat(88));
    for r in &rows {
        println!(
            "{:<28} {:>10} {:>9.1} {:>9.2} {:>9.2}× {:>9.2} {:>8.1}%",
            r.label,
            r.edges,
            r.synth_seconds,
            r.wall_seconds,
            r.realtime(),
            r.mean_hz,
            r.active_pct,
        );
    }
    println!();

    let best = rows
        .iter()
        .map(|r| r.realtime())
        .fold(f64::MIN, f64::max);
    println!("verdict");
    if best >= 1.0 {
        println!(
            "  Real time is reachable at {:.2}× — the simulator keeps up with the",
            best
        );
        println!("  wall clock on one core, and the other three are free for the browser.");
    } else {
        println!(
            "  Nothing here reaches real time (best {:.2}×). The fly will fly in slow",
            best
        );
        println!("  motion, or the next lever is needed: rayon across the leak loop.");
    }
    if let Some(mb) = peak_rss_mb() {
        println!("  Peak RSS {mb:.0} MB.");
    }
    Ok(())
}

fn print_vitals_warning(r: &Row) {
    let frac = r.total_spikes as f64 / (r.synth_seconds * 1000.0 * 139_255.0);
    if frac < 1e-5 {
        eprintln!(
            "  WARNING: essentially silent ({:.2e} spikes per neuron per ms). \
             The operating point is wrong, and these timings are for an idle loop.",
            frac
        );
    } else if frac > 0.5 {
        eprintln!(
            "  WARNING: saturating ({:.2} spikes per neuron per ms). \
             The operating point is wrong, and these timings are for a blown-up loop.",
            frac
        );
    } else {
        println!(
            "  operating point: {:.4} spikes per neuron per ms ({:.2} Hz mean)",
            frac,
            r.mean_hz
        );
    }
}

/// Report the per-tick cost of one configuration in isolation, for the README.
pub fn per_tick(c: &Circuit, params: Params, ticks: u64) -> f64 {
    let mut s = Sim::new(c, params, 3);
    let t0 = Instant::now();
    for _ in 0..ticks {
        s.step(c);
    }
    t0.elapsed().as_secs_f64() / ticks as f64
}

/// Kept so `bench` can be pointed at a specific file during development.
#[allow(dead_code)]
pub fn load(data_dir: &Path) -> Result<Circuit> {
    Circuit::load(&data_dir.join("brain.bin")).context("loading brain.bin")
}
