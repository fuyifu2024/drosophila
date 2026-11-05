//! Leaky integrate-and-fire over the whole connectome, 1 ms per tick.
//!
//! The kernel is a port of the one in `gnat` (which ran 668 neurons and was
//! tuned for the fly's escape circuit), widened to all 139,255. Everything
//! that was circuit-specific there — role baselines, the ×6 gap-junction
//! fudge, the loom input — is gone or moved out; what is left is the shape of
//! the update, which is the part worth keeping.
//!
//! The one piece of real circuit knowledge that survives is the **inhibitory
//! delay**. Excitatory synapses land on the target's membrane the instant the
//! presynaptic cell fires; inhibitory ones are held for 4 ms. That asymmetry is
//! why a fly can escape: the command to jump is a race between excitation
//! arriving now and inhibition arriving four milliseconds too late. Wiring both
//! with the same delay destroys the behaviour entirely.
//!
//! **These constants are a starting point, not a result.** They were tuned
//! against 668 neurons; 139,255 neurons with a mean in-degree of 19 have a very
//! different operating point, and `bench` exists to find out what that is
//! before anything downstream pretends to be a fly.

use crate::circuit::Circuit;
use crate::rng::Rng;

/// Milliseconds an inhibitory potential waits before it lands.
pub const INH_DELAY: usize = 4;

/// How often the per-neuron firing-rate estimate is decayed, in ticks.
/// Updating 139k rates every single millisecond would cost as much as the
/// membrane update itself, and a 120 ms rate does not need 1 ms resolution.
pub const RATE_STRIDE: usize = 16;

/// Time constant of the rate estimate, in milliseconds.
pub const RATE_TAU_MS: f32 = 120.0;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Membrane leak per millisecond. 0.9512 is τ = 20 ms.
    pub decay: f32,
    pub threshold: f32,
    pub refractory_ms: u8,
    /// Converts a synapse count into a membrane step. The single knob that sets
    /// how excitable the whole circuit is.
    pub weight_scale: f32,
    /// Probability per neuron per millisecond of a random kick.
    pub p_noise: f32,
    pub noise_kick: f32,
    /// Floor on the membrane potential, so a neuron that takes a burst of
    /// inhibition is not pushed so far down that it can never fire again.
    pub v_floor: f32,
    /// Baseline drive, drawn uniformly per neuron at construction. Steady state
    /// is `baseline / (1 - decay)`, i.e. about 20.5× the baseline.
    pub baseline_lo: f32,
    pub baseline_hi: f32,
}

impl Default for Params {
    fn default() -> Self {
        // Ported from gnat's constants, with the baseline range widened because
        // there is no per-role grouping here to set it any finer.
        Params {
            decay: 0.9512,
            threshold: 1.0,
            refractory_ms: 2,
            weight_scale: 0.0008,
            p_noise: 0.0022,
            noise_kick: 0.42,
            v_floor: -2.0,
            baseline_lo: 0.005,
            baseline_hi: 0.035,
        }
    }
}

pub struct Sim {
    pub n: usize,
    pub v: Vec<f32>,
    pub refr: Vec<u8>,
    pub baseline: Vec<f32>,
    /// Four delay lines, one per millisecond of the inhibitory hold. Kept as
    /// four separate arrays rather than `Vec<[f32; 4]>` so delivering the
    /// current millisecond's inhibition is a linear walk over 556 KB instead of
    /// a strided gather over 2.2 MB.
    pub inh: [Vec<f32>; INH_DELAY],
    /// Spike-count accumulator for the rate estimate; decays every
    /// `RATE_STRIDE` ticks. Multiply by `1000 / RATE_TAU_MS` for Hz.
    pub rate: Vec<f32>,
    pub rng: Rng,
    /// Neurons that fired on the most recent tick.
    pub fired: Vec<u32>,
    pub tick: u64,
    pub params: Params,
    /// Total spikes since construction, for the invariant tests.
    pub total_spikes: u64,
}

impl Sim {
    pub fn new(c: &Circuit, params: Params, seed: u64) -> Self {
        let n = c.n();
        let mut rng = Rng::from_seed(seed);
        let baseline = (0..n)
            .map(|_| rng.range(params.baseline_lo, params.baseline_hi))
            .collect();

        Sim {
            n,
            v: vec![0.0; n],
            refr: vec![0; n],
            baseline,
            inh: std::array::from_fn(|_| vec![0.0; n]),
            rate: vec![0.0; n],
            rng,
            fired: Vec::with_capacity(n / 64),
            tick: 0,
            params,
            total_spikes: 0,
        }
    }

    /// Add current to one neuron, before the next tick's leak. This is how the
    /// senses drive the brain: not by overwriting state, but by pushing a
    /// specific, named population of real neurons.
    #[inline]
    pub fn stimulate(&mut self, i: usize, amount: f32) {
        self.v[i] += amount;
    }

    /// Advance one millisecond.
    #[inline]
    pub fn step(&mut self, c: &Circuit) {
        let n = self.n;
        let site = (self.tick as usize) % INH_DELAY;

        // --- 1. leak, accept whatever arrives, spontaneous kick, threshold ---
        // Taken out of `self` so the propagation loop below can hold `&mut
        // self.v` without fighting the borrow checker, and put back at the end
        // so the allocation is reused every tick.
        let mut fired = std::mem::take(&mut self.fired);
        fired.clear();
        {
            let Params {
                decay,
                threshold,
                refractory_ms,
                p_noise,
                noise_kick,
                v_floor,
                ..
            } = self.params;
            let v = &mut self.v;
            let refr = &mut self.refr;
            let baseline = &self.baseline;
            let arriving = &self.inh[site];
            let rng = &mut self.rng;
            for i in 0..n {
                // Inhibition scheduled INH_DELAY ms ago lands *after* the leak,
                // so a potential that has just travelled the delay line is not
                // also attenuated by the tick it arrives on. Excitation is
                // added the same way, in the propagation step below.
                let mut vi = v[i] * decay + arriving[i] + baseline[i];
                if refr[i] > 0 {
                    // A refractory neuron still leaks and still receives, it
                    // just cannot spike. Skipping it entirely would silently
                    // drop whatever arrived during its refractory window.
                    refr[i] -= 1;
                } else {
                    if rng.next_f32() < p_noise {
                        vi += noise_kick;
                    }
                    if vi >= threshold {
                        vi = 0.0;
                        refr[i] = refractory_ms;
                        fired.push(i as u32);
                    }
                }
                if vi < v_floor {
                    vi = v_floor;
                }
                v[i] = vi;
            }
        }
        // The delay line has been read; it now becomes the slot that this
        // tick's inhibitory spikes are written into, which is what makes them
        // arrive exactly INH_DELAY ticks from now.
        self.inh[site].iter_mut().for_each(|x| *x = 0.0);

        // --- 2. propagate ---
        // Excitatory: straight onto the target's membrane. Inhibitory: into the
        // delay line slot that will be read exactly INH_DELAY ticks from now.
        {
            let scale = self.params.weight_scale;
            let v = &mut self.v;
            let inh = &mut self.inh[site];
            for &src in &fired {
                let a = c.row_start[src as usize] as usize;
                let b = c.row_start[src as usize + 1] as usize;
                for k in a..b {
                    let w = c.weight[k] * scale;
                    let t = c.col_idx[k] as usize;
                    if w >= 0.0 {
                        v[t] += w;
                    } else {
                        inh[t] += w;
                    }
                }
            }
        }

        // --- 3. rate estimate, amortised ---
        for &src in &fired {
            self.rate[src as usize] += 1.0;
        }
        if self.tick as usize % RATE_STRIDE == RATE_STRIDE - 1 {
            let d = (1.0 - 1.0 / RATE_TAU_MS).powi(RATE_STRIDE as i32);
            self.rate.iter_mut().for_each(|r| *r *= d);
        }

        self.total_spikes += fired.len() as u64;
        self.fired = fired;
        self.tick += 1;
    }

    /// Firing rate of one neuron, in Hz.
    #[inline]
    pub fn hz(&self, i: usize) -> f32 {
        self.rate[i] * (1000.0 / RATE_TAU_MS)
    }

    /// Mean firing rate over a set of neurons, in Hz. This is how everything
    /// downstream reads the brain: a population, not a cell.
    pub fn group_hz(&self, idx: &[u32]) -> f32 {
        if idx.is_empty() {
            return 0.0;
        }
        let sum: f32 = idx.iter().map(|&i| self.rate[i as usize]).sum();
        sum * (1000.0 / RATE_TAU_MS) / idx.len() as f32
    }

    /// A compact picture of what the circuit is doing, for the benchmark and
    /// the invariant tests.
    pub fn vitals(&self) -> Vitals {
        let (mut silent, mut active) = (0usize, 0usize);
        let mut max_hz = 0.0f32;
        for i in 0..self.n {
            let h = self.hz(i);
            if h < 0.5 {
                silent += 1;
            } else {
                active += 1;
            }
            if h > max_hz {
                max_hz = h;
            }
        }
        Vitals {
            tick: self.tick,
            total_spikes: self.total_spikes,
            mean_hz: self.hz_mean(),
            max_hz,
            active,
            silent,
        }
    }

    pub fn hz_mean(&self) -> f32 {
        let sum: f32 = self.rate.iter().sum();
        sum * (1000.0 / RATE_TAU_MS) / self.n as f32
    }

    /// Fraction of the population that fired on the most recent tick.
    pub fn active_fraction(&self) -> f32 {
        self.fired.len() as f32 / self.n as f32
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Vitals {
    pub tick: u64,
    pub total_spikes: u64,
    pub mean_hz: f32,
    pub max_hz: f32,
    pub active: usize,
    pub silent: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::etl::{MAGIC, NEURON_RECORD, VERSION};

    /// Build a circuit in memory: `n` neurons, laid out exactly as `etl` writes
    /// them. Edges are (from, to, weight).
    fn circuit(n: usize, edges: &[(usize, usize, f32)]) -> Circuit {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&(n as u32).to_le_bytes());
        b.extend_from_slice(&(edges.len() as u32).to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        for i in 0..n {
            b.extend_from_slice(&(i as u64).to_le_bytes());
            for v in [i as f32, 0.0, 0.0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[0, 0, 0, 1]);
            b.extend_from_slice(&[(i % 21) as u8, 0, 0, 0]);
        }
        let mut deg = vec![0u32; n];
        for &(a, _, _) in edges {
            deg[a] += 1;
        }
        let mut acc = 0u32;
        for d in &deg {
            b.extend_from_slice(&acc.to_le_bytes());
            acc += d;
        }
        b.extend_from_slice(&acc.to_le_bytes());
        for &(_, t, _) in edges {
            b.extend_from_slice(&(t as u32).to_le_bytes());
        }
        for &(_, _, w) in edges {
            b.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(b.len(), 24 + n * NEURON_RECORD + (n + 1) * 4 + edges.len() * 8);
        let c = Circuit::from_bytes(&b).unwrap();
        c.validate().unwrap();
        c
    }

    fn quiet(n: usize, edges: &[(usize, usize, f32)]) -> (Circuit, Sim) {
        let c = circuit(n, edges);
        let mut p = Params::default();
        p.p_noise = 0.0;
        p.baseline_lo = 0.0;
        p.baseline_hi = 0.0;
        let s = Sim::new(&c, p, 1);
        (c, s)
    }

    #[test]
    fn a_neuron_with_no_input_never_fires() {
        let (c, mut s) = quiet(4, &[]);
        for _ in 0..1000 {
            s.step(&c);
        }
        assert_eq!(s.total_spikes, 0);
        assert!(s.fired.is_empty());
    }

    #[test]
    fn excitation_lands_on_the_same_tick() {
        let (c, mut s) = quiet(2, &[(0, 1, 1000.0)]);
        s.v[0] = 2.0; // 2.0 × 0.9512 = 1.90, comfortably over a threshold of 1.0
        s.step(&c);
        assert_eq!(s.fired, vec![0u32], "neuron 0 should have fired");
        // weight 1000 × scale 0.0008 = 0.8, immediately on neuron 1's membrane.
        assert!(
            (s.v[1] - 0.8).abs() < 1e-5,
            "excitation should land this tick, got {}",
            s.v[1]
        );
    }

    #[test]
    fn inhibition_lands_exactly_inh_delay_ticks_later() {
        let (c, mut s) = quiet(2, &[(0, 1, -1000.0)]);
        s.v[0] = 2.0;
        s.step(&c);
        assert_eq!(s.fired, vec![0u32]);
        assert_eq!(s.v[1], 0.0, "inhibition must not land on the tick it was emitted");
        for _ in 0..INH_DELAY - 1 {
            s.step(&c);
            assert_eq!(s.v[1], 0.0, "inhibition arrived early");
        }
        s.step(&c);
        assert!(
            (s.v[1] + 0.8).abs() < 1e-5,
            "inhibition should land on tick {INH_DELAY}, got {}",
            s.v[1]
        );
    }

    #[test]
    fn a_fired_neuron_is_reset_and_refractory() {
        let (c, mut s) = quiet(1, &[]);
        s.v[0] = 1.5;
        s.step(&c);
        assert_eq!(s.fired, vec![0u32]);
        assert_eq!(s.v[0], 0.0);
        // A second spike inside the refractory window must be impossible.
        s.v[0] = 1.5;
        s.step(&c);
        assert!(s.fired.is_empty(), "fired during the refractory period");
    }

    #[test]
    fn the_rate_estimate_tracks_a_known_firing_rate() {
        let (c, mut s) = quiet(1, &[]);
        // Stimulate every 4th tick, then let the rate settle over several time
        // constants and check it reads back in Hz.
        for t in 0..4000 {
            if t % 4 == 0 {
                s.v[0] = 1.5;
            }
            s.step(&c);
        }
        let hz = s.hz(0);
        assert!(
            (hz - 250.0).abs() < 40.0,
            "expected about 250 Hz (one spike every 4 ms), got {hz}"
        );
    }

    #[test]
    fn the_same_seed_reproduces_the_same_spikes() {
        // This is the only kind of test a 139k-neuron circuit admits: nothing
        // is deterministic per-cell, so we pin the seed instead.
        let edges: Vec<_> = (0..64).map(|i| (i, (i * 7 + 3) % 64, 0.4)).collect();
        let c = circuit(64, &edges);
        let run = |seed| {
            let mut s = Sim::new(&c, Params::default(), seed);
            for _ in 0..500 {
                s.step(&c);
            }
            (s.total_spikes, (s.rate.iter().sum::<f32>() * 1000.0).round() as i64)
        };
        assert_eq!(run(42), run(42));
        assert_ne!(run(42), run(43));
    }

    #[test]
    fn a_runaway_circuit_stays_finite() {
        // Every neuron drives every other with a huge weight: without the
        // membrane floor and the refractory period this diverges to infinity.
        let n = 32;
        let edges: Vec<_> = (0..n)
            .flat_map(|i| (0..n).map(move |j| (i, j, 500.0)))
            .collect();
        let c = circuit(n, &edges);
        let mut s = Sim::new(&c, Params::default(), 9);
        for _ in 0..2000 {
            s.step(&c);
        }
        // These two are guaranteed by the update itself.
        assert!(s.v.iter().all(|x| x.is_finite()), "membrane potential diverged");
        assert!(s.v.iter().all(|&x| x >= -2.001), "membrane floor breached");
        // Saturating the circuit is the *expected* outcome of this input, and
        // the membrane legitimately sits well above threshold between spikes.
        // What must not happen is the firing rate escaping the refractory
        // ceiling — with a 2 ms refractory that is one spike per 3 ticks.
        let hz = s.hz(0);
        assert!(
            hz <= 1000.0 / 3.0 + 20.0,
            "the refractory period did not bound the rate: {hz} Hz"
        );
        assert!(hz > 100.0, "a maximally driven neuron should be firing hard, got {hz} Hz");
    }
}
