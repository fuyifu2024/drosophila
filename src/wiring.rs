//! The handles the rest of the program has on the brain.
//!
//! Everything outside the simulator addresses the connectome through this
//! struct and nothing else. That is deliberate: it is the complete list of
//! places where this project claims to know what a neuron does, and every
//! entry is a real annotated cell type rather than an index range.
//!
//! Reading the list also shows what is *not* claimed. There is no handle for
//! "the fly's intention", no handle for memory, no handle for the mushroom
//! body beyond counting its Kenyon cells. Sensory neurons go in, descending
//! neurons come out, and what happens between them is whatever the wiring
//! does.

use crate::circuit::Circuit;
use crate::etl::{
    G_ASCENDING, G_CENTRAL_COMPLEX, G_DESCENDING, G_LC4, G_LPLC2, G_MOTOR, G_PHOTORECEPTOR, G_T4A,
    G_T4B, G_T4C, G_T4D, G_T5A, G_T5B, G_T5C, G_T5D,
};
use crate::etl::{SIDE_LEFT, SIDE_RIGHT};

pub struct Wiring {
    /// R1-6, R7, R8. Split by eye so the fly can tell which side is brighter —
    /// which is the whole basis of its phototaxis.
    pub photoreceptor_left: Vec<u32>,
    pub photoreceptor_right: Vec<u32>,
    /// The four horizontal/vertical motion channels, ON-edge (T4) and OFF-edge
    /// (T5). Within each array the four entries are the four cell types a–d.
    pub t4: [Vec<u32>; 4],
    pub t5: [Vec<u32>; 4],
    /// Looming detectors: the collision alarm.
    pub looming: Vec<u32>,
    /// Everything that leaves the brain for the nerve cord, split by side.
    pub descending_left: Vec<u32>,
    pub descending_right: Vec<u32>,
    /// Counts kept for the report, so it is visible how much of the brain each
    /// handle actually addresses.
    pub counts: WiringCounts,
}

#[derive(Debug, Clone, Copy)]
pub struct WiringCounts {
    pub photoreceptor: usize,
    pub motion: usize,
    pub looming: usize,
    pub descending: usize,
    pub ascending: usize,
    pub central_complex: usize,
}

impl WiringCounts {
    pub fn total(&self) -> usize {
        self.photoreceptor + self.motion + self.looming + self.descending + self.ascending
    }
}

fn split_by_side(c: &Circuit, idx: Vec<u32>) -> (Vec<u32>, Vec<u32>) {
    let (mut l, mut r) = (Vec::new(), Vec::new());
    for i in idx {
        match c.side[i as usize] {
            SIDE_LEFT => l.push(i),
            SIDE_RIGHT => r.push(i),
            _ => {
                // Unlabelled centres go to whichever side they sit on in the
                // world frame, where +y is the animal's left.
                if c.pos[i as usize][1] >= 0.0 {
                    l.push(i)
                } else {
                    r.push(i)
                }
            }
        }
    }
    (l, r)
}

impl Wiring {
    pub fn new(c: &Circuit) -> Self {
        let (photoreceptor_left, photoreceptor_right) =
            split_by_side(c, c.by_group(G_PHOTORECEPTOR));

        let t4 = [
            c.by_group(G_T4A),
            c.by_group(G_T4B),
            c.by_group(G_T4C),
            c.by_group(G_T4D),
        ];
        let t5 = [
            c.by_group(G_T5A),
            c.by_group(G_T5B),
            c.by_group(G_T5C),
            c.by_group(G_T5D),
        ];

        let mut looming = c.by_group(G_LPLC2);
        looming.extend(c.by_group(G_LC4));

        let (descending_left, descending_right) = split_by_side(c, c.by_group(G_DESCENDING));

        let motion = t4.iter().map(|v| v.len()).sum::<usize>()
            + t5.iter().map(|v| v.len()).sum::<usize>();

        let counts = WiringCounts {
            photoreceptor: photoreceptor_left.len() + photoreceptor_right.len(),
            motion,
            looming: looming.len(),
            descending: descending_left.len() + descending_right.len(),
            ascending: c.by_group(G_ASCENDING).len(),
            central_complex: c.by_group(G_CENTRAL_COMPLEX).len(),
        };
        // G_MOTOR is part of the output layer too; count it into `descending`
        // so the report shows the true size of the brain's output.
        let counts = WiringCounts {
            descending: counts.descending + c.by_group(G_MOTOR).len(),
            ..counts
        };

        Wiring {
            photoreceptor_left,
            photoreceptor_right,
            t4,
            t5,
            looming,
            descending_left,
            descending_right,
            counts,
        }
    }

    /// Every neuron this program ever writes to, as one flat list. Used to
    /// assert that the senses are not quietly driving half the brain.
    pub fn all_driven(&self) -> Vec<u32> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.photoreceptor_left);
        v.extend_from_slice(&self.photoreceptor_right);
        for g in self.t4.iter().chain(self.t5.iter()) {
            v.extend_from_slice(g);
        }
        v.extend_from_slice(&self.looming);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitting_by_side_keeps_every_neuron() {
        // A circuit with one neuron per side plus one unlabelled centre.
        let mut b = Vec::new();
        b.extend_from_slice(&crate::etl::MAGIC.to_le_bytes());
        b.extend_from_slice(&crate::etl::VERSION.to_le_bytes());
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        let recs: [(u64, [f32; 3], [u8; 5]); 3] = [
            (1, [0.0, 5.0, 0.0], [0, SIDE_LEFT, 0, 1, G_PHOTORECEPTOR]),
            (2, [0.0, -5.0, 0.0], [0, SIDE_RIGHT, 0, 1, G_PHOTORECEPTOR]),
            (3, [0.0, 0.0, 0.0], [0, 3, 0, 1, G_PHOTORECEPTOR]),
        ];
        for (id, p, meta) in recs {
            b.extend_from_slice(&id.to_le_bytes());
            for v in p {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&meta);
            b.extend_from_slice(&[0, 0, 0]);
        }
        for v in [0u32, 0, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let c = Circuit::from_bytes(&b).unwrap();
        c.validate().unwrap();

        let w = Wiring::new(&c);
        assert_eq!(w.photoreceptor_left.len(), 2, "left + unlabelled centre");
        assert_eq!(w.photoreceptor_right.len(), 1);
        assert_eq!(
            w.photoreceptor_left.len() + w.photoreceptor_right.len(),
            3,
            "no neuron may be lost in the split"
        );
    }
}
