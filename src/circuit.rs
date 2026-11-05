//! Reading `brain.bin` back, and checking that what came out of the ETL is
//! actually a connectome rather than 35 MB of plausible-looking noise.
//!
//! The format is deliberately mmap-friendly: a 24-byte header, a fixed-stride
//! neuron table, then three flat CSR arrays. Nothing here allocates per
//! neuron, because at 139k neurons that is the difference between a 0.2 s load
//! and a 20 s one.

use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::coord;
use crate::etl::{HEADER_BYTES, MAGIC, NEURON_RECORD, VERSION};

pub struct Circuit {
    pub root_id: Vec<u64>,
    /// Brain units, not millimetres — the viewer converts at the last moment.
    pub pos: Vec<[f32; 3]>,
    // The per-neuron annotations. Not every one is read yet: `side` and `flow`
    // are what `motor` selects the output neurons by, and `class` is what the
    // viewer colours by. They are loaded here so the layout stays in one file.
    #[allow(dead_code)]
    pub class: Vec<u8>,
    #[allow(dead_code)]
    pub side: Vec<u8>,
    #[allow(dead_code)]
    pub flow: Vec<u8>,
    #[allow(dead_code)]
    pub flags: Vec<u8>,
    /// Named cell group, index into `etl::GROUP_NAMES`. This is how the wiring
    /// finds the photoreceptors and the descending neurons.
    pub group: Vec<u8>,
    /// CSR, length n + 1.
    pub row_start: Vec<u32>,
    pub col_idx: Vec<u32>,
    /// Signed synapse count, already merged per (pre, post).
    pub weight: Vec<f32>,
    pub declared_synapses: u64,
}

impl Circuit {
    pub fn n(&self) -> usize {
        self.root_id.len()
    }

    pub fn n_edges(&self) -> usize {
        self.col_idx.len()
    }

    /// Out-neighbours of `i` as (target, signed weight) pairs. This is the
    /// inner loop of the whole simulation.
    #[inline]
    #[allow(dead_code)]
    pub fn out_edges(&self, i: usize) -> impl Iterator<Item = (u32, f32)> + '_ {
        let a = self.row_start[i] as usize;
        let b = self.row_start[i + 1] as usize;
        (a..b).map(move |k| (self.col_idx[k], self.weight[k]))
    }

    pub fn out_degree(&self, i: usize) -> usize {
        (self.row_start[i + 1] - self.row_start[i]) as usize
    }

    /// Indices of every neuron with the given flow code, in ascending order.
    /// This is how the motor layer finds the brain's descending output.
    #[allow(dead_code)]
    pub fn by_flow(&self, flow: u8) -> Vec<u32> {
        (0..self.n() as u32)
            .filter(|&i| self.flow[i as usize] == flow)
            .collect()
    }

    #[allow(dead_code)]
    pub fn by_class(&self, class: u8) -> Vec<u32> {
        (0..self.n() as u32)
            .filter(|&i| self.class[i as usize] == class)
            .collect()
    }

    /// Indices of every neuron in a named cell group, in ascending order.
    ///
    /// This is the only handle the rest of the program has on the brain: the
    /// senses drive `G_PHOTORECEPTOR` and the `T4`/`T5` motion detectors, and
    /// the body is steered by `G_DESCENDING`. Everything else is left to be
    /// whatever the connectome makes of it.
    pub fn by_group(&self, group: u8) -> Vec<u32> {
        (0..self.n() as u32)
            .filter(|&i| self.group[i as usize] == group)
            .collect()
    }

    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::from_bytes(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_BYTES {
            bail!("file is {} bytes, shorter than the {HEADER_BYTES}-byte header", b.len());
        }
        let magic = u32::from_le_bytes(b[0..4].try_into().unwrap());
        if magic != MAGIC {
            bail!("bad magic {magic:#010x}, expected {MAGIC:#010x}");
        }
        let version = u32::from_le_bytes(b[4..8].try_into().unwrap());
        if version != VERSION {
            bail!("format version {version}, this build speaks {VERSION}");
        }
        let n = u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize;
        let n_edges = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let declared_synapses = u64::from_le_bytes(b[16..24].try_into().unwrap());

        let table_end = HEADER_BYTES + n * NEURON_RECORD;
        let csr_bytes = (n + 1) * 4 + n_edges * 4 + n_edges * 4;
        let need = table_end + csr_bytes;
        if b.len() < need {
            bail!(
                "file is {} bytes but the header implies {need} ({} neurons, {} edges)",
                b.len(),
                n,
                n_edges
            );
        }

        let mut root_id = Vec::with_capacity(n);
        let mut pos = Vec::with_capacity(n);
        let mut class = Vec::with_capacity(n);
        let mut side = Vec::with_capacity(n);
        let mut flow = Vec::with_capacity(n);
        let mut flags = Vec::with_capacity(n);
        let mut group = Vec::with_capacity(n);
        for i in 0..n {
            let o = HEADER_BYTES + i * NEURON_RECORD;
            root_id.push(u64::from_le_bytes(b[o..o + 8].try_into().unwrap()));
            let x = f32::from_le_bytes(b[o + 8..o + 12].try_into().unwrap());
            let y = f32::from_le_bytes(b[o + 12..o + 16].try_into().unwrap());
            let z = f32::from_le_bytes(b[o + 16..o + 20].try_into().unwrap());
            pos.push([x, y, z]);
            class.push(b[o + 20]);
            side.push(b[o + 21]);
            flow.push(b[o + 22]);
            flags.push(b[o + 23]);
            group.push(b[o + 24]);
        }

        let mut o = table_end;
        let mut row_start = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            row_start.push(u32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
            o += 4;
        }
        let mut col_idx = Vec::with_capacity(n_edges);
        for _ in 0..n_edges {
            col_idx.push(u32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
            o += 4;
        }
        let mut weight = Vec::with_capacity(n_edges);
        for _ in 0..n_edges {
            weight.push(f32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
            o += 4;
        }

        Ok(Circuit {
            root_id,
            pos,
            class,
            side,
            flow,
            flags,
            group,
            row_start,
            col_idx,
            weight,
            declared_synapses,
        })
    }

    /// A copy with every edge weaker than `min_abs` synapses removed.
    ///
    /// The full connectome has a long tail of one-synapse connections — real,
    /// but individually almost meaningless, and they are most of the 2.66M
    /// edges. Dropping them is the first lever if the simulator turns out not
    /// to run in real time, and it is deliberately a lever rather than a
    /// silent default: whatever it costs in fidelity gets written down.
    pub fn pruned(&self, min_abs: f32) -> Circuit {
        let n = self.n();
        let mut row_start = vec![0u32; n + 1];
        let mut col_idx = Vec::with_capacity(self.n_edges());
        let mut weight = Vec::with_capacity(self.n_edges());
        for i in 0..n {
            let a = self.row_start[i] as usize;
            let b = self.row_start[i + 1] as usize;
            for k in a..b {
                if self.weight[k].abs() >= min_abs {
                    col_idx.push(self.col_idx[k]);
                    weight.push(self.weight[k]);
                }
            }
            row_start[i + 1] = col_idx.len() as u32;
        }
        Circuit {
            root_id: self.root_id.clone(),
            pos: self.pos.clone(),
            class: self.class.clone(),
            side: self.side.clone(),
            flow: self.flow.clone(),
            flags: self.flags.clone(),
            group: self.group.clone(),
            row_start,
            col_idx,
            weight,
            declared_synapses: self.declared_synapses,
        }
    }

    /// Structural checks. Anything wrong here makes every downstream number
    /// meaningless, so this runs before anything else touches the circuit.
    pub fn validate(&self) -> Result<()> {
        let n = self.n();
        if self.row_start.len() != n + 1 {
            bail!("row_start has {} entries, expected {}", self.row_start.len(), n + 1);
        }
        if self.row_start[0] != 0 {
            bail!("row_start[0] = {}, must be 0", self.row_start[0]);
        }
        if *self.row_start.last().unwrap() as usize != self.n_edges() {
            bail!(
                "row_start ends at {} but there are {} edges",
                self.row_start.last().unwrap(),
                self.n_edges()
            );
        }
        for i in 0..n {
            if self.row_start[i] > self.row_start[i + 1] {
                bail!("row_start not monotonic at neuron {i}");
            }
        }
        let mut max_target = 0u32;
        let mut signs = [0usize; 3];
        for &c in &self.col_idx {
            if c as usize >= n {
                bail!("edge target {c} is out of range for {n} neurons");
            }
            max_target = max_target.max(c);
        }
        for &w in &self.weight {
            if !w.is_finite() {
                bail!("non-finite weight {w}");
            }
            signs[if w > 0.0 {
                0
            } else if w < 0.0 {
                1
            } else {
                2
            }] += 1;
        }
        if signs[2] > 0 {
            bail!("{} edges have exactly zero weight", signs[2]);
        }
        // A whole connectome in which nothing reaches the far end of the index
        // space is a sign the ETL matched the wrong ids. Gated on size because
        // the heuristic is meaningless for the handful-of-neurons circuits the
        // unit tests build.
        if n >= 10_000 && (max_target as usize) < n / 2 {
            bail!("highest edge target is {max_target} in a {n}-neuron circuit — suspicious");
        }
        Ok(())
    }

    /// Everything worth knowing about the shape of the circuit, for `stats`.
    pub fn report(&self) -> String {
        let n = self.n();
        let e = self.n_edges();
        let mut s = String::new();

        let mut deg_min = usize::MAX;
        let mut deg_max = 0usize;
        let mut isolates = 0usize;
        let mut sum_sq = 0u64;
        for i in 0..n {
            let d = self.out_degree(i);
            deg_min = deg_min.min(d);
            deg_max = deg_max.max(d);
            if d == 0 {
                isolates += 1;
            }
            sum_sq += (d as u64) * (d as u64);
        }
        let mean = e as f64 / n as f64;
        let var = (sum_sq as f64 / n as f64) - mean * mean;
        s.push_str(&format!("neurons                 {n}\n"));
        s.push_str(&format!("edges                   {e}\n"));
        s.push_str(&format!("declared synapses       {}\n", self.declared_synapses));
        s.push_str(&format!(
            "out-degree              min {deg_min}  max {deg_max}  mean {mean:.2}  sd {:.2}  isolates {isolates}\n",
            var.max(0.0).sqrt()
        ));

        let (mut pos_w, mut neg_w) = (0u64, 0u64);
        let (mut pos_s, mut neg_s) = (0u64, 0u64);
        let mut max_abs = 0.0f32;
        for &w in &self.weight {
            if w > 0.0 {
                pos_w += 1;
                pos_s += w as u64;
            } else {
                neg_w += 1;
                neg_s += (-w) as u64;
            }
            max_abs = max_abs.max(w.abs());
        }
        s.push_str(&format!(
            "excitatory edges        {pos_w}  ({pos_s} synapses)\n"
        ));
        s.push_str(&format!(
            "inhibitory edges        {neg_w}  ({neg_s} synapses)\n"
        ));
        s.push_str(&format!("largest |weight|        {max_abs:.0}\n"));

        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &self.pos {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        s.push_str(&format!(
            "bbox (brain units)      x {:.2}..{:.2}  y {:.2}..{:.2}  z {:.2}..{:.2}\n",
            lo[0], hi[0], lo[1], hi[1], lo[2], hi[2]
        ));
        s.push_str(&format!(
            "world size (mm)         {:.3} long × {:.3} wide × {:.3} tall\n",
            (hi[0] - lo[0]) as f64 * coord::MM_PER_BRAIN_UNIT as f64,
            (hi[1] - lo[1]) as f64 * coord::MM_PER_BRAIN_UNIT as f64,
            (hi[2] - lo[2]) as f64 * coord::MM_PER_BRAIN_UNIT as f64
        ));

        // The one check that catches a mirrored or rotated brain. Neurons that
        // leave the brain for the nerve cord must sit at the back of it, so
        // their mean world x has to be clearly negative. If a sign ever flips,
        // every other number in this report still looks fine and this one does
        // not.
        s.push_str("\nmean position by flow (world mm; x = anterior, y = left, z = dorsal)\n");
        let names = ["intrinsic", "afferent", "efferent", "unknown"];
        let mut efferent_x = 0.0f32;
        for (code, name) in names.iter().enumerate() {
            let idx: Vec<usize> = (0..n).filter(|&i| self.flow[i] == code as u8).collect();
            if idx.is_empty() {
                continue;
            }
            let mut m = [0.0f64; 3];
            for &i in &idx {
                for k in 0..3 {
                    m[k] += self.pos[i][k] as f64 * coord::MM_PER_BRAIN_UNIT as f64;
                }
            }
            let mean = [m[0] / idx.len() as f64, m[1] / idx.len() as f64, m[2] / idx.len() as f64];
            s.push_str(&format!(
                "  {name:<12} n={:<7} x {:+.3}  y {:+.3}  z {:+.3}\n",
                idx.len(),
                mean[0],
                mean[1],
                mean[2]
            ));
            if code == 2 {
                efferent_x = mean[0] as f32;
            }
        }
        s.push_str(&format!(
            "  → efferent neurons sit {:+.3} mm along x; they must be behind the\n    \
             centre, because that is the direction of the nerve cord.\n",
            efferent_x
        ));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two neurons, one inhibitory edge from 0 to 1 — written out in the exact
    /// byte layout `etl` produces, so a change to one side of the format cannot
    /// silently break the other.
    fn synthetic() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&7u64.to_le_bytes());
        // neuron 0
        b.extend_from_slice(&111u64.to_le_bytes());
        for v in [1.0f32, 2.0, 3.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[4, 1, 2, 1]);
        b.extend_from_slice(&[1, 0, 0, 0]); // group 1, three reserved bytes
        // neuron 1
        b.extend_from_slice(&222u64.to_le_bytes());
        for v in [-1.0f32, 0.0, 0.5] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0, 0, 0, 1]);
        b.extend_from_slice(&[1, 0, 0, 0]);
        // CSR
        for v in [0u32, 1, 1] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(-7.0f32).to_le_bytes());
        b
    }

    #[test]
    fn a_handmade_circuit_survives_the_round_trip() {
        let b = synthetic();
        let c = Circuit::from_bytes(&b).unwrap();
        c.validate().unwrap();
        assert_eq!(c.n(), 2);
        assert_eq!(c.n_edges(), 1);
        assert_eq!(c.root_id[0], 111);
        assert_eq!(c.pos[1], [-1.0, 0.0, 0.5]);
        assert_eq!(c.class[0], 4);
        assert_eq!(c.flow[1], 0);
        assert_eq!(c.group[1], 1);
        let e: Vec<_> = c.out_edges(0).collect();
        assert_eq!(e, vec![(1u32, -7.0f32)]);
        assert_eq!(c.out_edges(1).count(), 0);
        assert_eq!(c.out_degree(0), 1);
    }

    #[test]
    fn a_truncated_file_is_rejected_rather_than_read_as_garbage() {
        // A partial download is the likeliest way this file arrives broken, and
        // reading it as if it were whole would give plausible wrong answers
        // rather than an error.
        let mut b = synthetic();
        b.truncate(b.len() - 4);
        match Circuit::from_bytes(&b) {
            Ok(_) => panic!("a file with its last edge missing parsed successfully"),
            Err(e) => assert!(format!("{e}").contains("header implies"), "{e}"),
        }
    }

    #[test]
    fn a_file_that_is_not_ours_is_rejected() {
        let mut b = synthetic();
        b[0] ^= 0xff;
        match Circuit::from_bytes(&b) {
            Ok(_) => panic!("a file with the wrong magic parsed successfully"),
            Err(e) => assert!(format!("{e}").contains("bad magic"), "{e}"),
        }
    }

    #[test]
    fn an_out_of_range_edge_target_is_caught() {
        let mut b = synthetic();
        // The one edge's target sits just past the neuron table and the
        // three-entry row_start array. Walking to it by hand also pins the
        // record stride, so a change to NEURON_RECORD breaks this test rather
        // than silently reading the wrong bytes.
        let at = HEADER_BYTES + 2 * NEURON_RECORD + 3 * 4;
        assert_eq!(u32::from_le_bytes(b[at..at + 4].try_into().unwrap()), 1);
        b[at..at + 4].copy_from_slice(&9u32.to_le_bytes());
        let c = Circuit::from_bytes(&b).unwrap();
        match c.validate() {
            Ok(()) => panic!("an out-of-range edge target validated"),
            Err(e) => assert!(format!("{e}").contains("out of range"), "{e}"),
        }
    }
}
