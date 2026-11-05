//! Coordinate frames.
//!
//! Every conversion between frames lives in this file and nowhere else. Get one
//! sign wrong and the fly has its brain installed backwards, which is the kind
//! of bug that survives all the way to a screenshot before anyone notices.
//!
//! # The FlyWire frame
//!
//! FAFB v783 is one FIB-SEM volume of one adult female brain. Its axes are not
//! documented anywhere convenient, so they were established from the annotation
//! itself:
//!
//! * **x — left / right.** The paired neuropils separate on this axis: the
//!   antennal lobes labelled `AL_L` and `AL_R` sit at x = 581,000 and
//!   x = 470,000 nm. Higher x is the animal's **left**.
//!
//! * **y — anterior / posterior.** Higher y is **posterior**. Four independent
//!   nerve-based signals say so: the cervical connective (`CV`, 3,687 neurons)
//!   sits at y = 311,000; the descending and ascending neurons that run to the
//!   nerve cord at y = 288,000 and 326,000; the gnathal ganglion at 318,000.
//!   The clincher is a single structure disagreeing with itself: the mushroom
//!   body's **vertical lobe**, its anterior end, sits at y = 126,000 while its
//!   own **calyx**, the posterior end, sits at y = 142,000.
//!
//! * **z — ventral / dorsal.** Higher z is **dorsal**. The antennal lobe, a
//!   ventral structure, is at z = 43,000; the mushroom body calyx at 174,000.
//!   The central complex orders itself correctly as well — ellipsoid body
//!   (ventral) 89,000 < fan-shaped body 118,000 < protocerebral bridge
//!   (dorsal) 176,000.
//!
//! The raw frame is right-handed, as (left, posterior, dorsal).
//!
//! # The world frame
//!
//! Right-handed millimetres, chosen so the fly is easy to think about:
//!
//! * **+x — anterior.** The fly faces +x; so does the brain's long axis.
//! * **+y — the animal's left.**
//! * **+z — dorsal.**
//!
//! That is the raw frame rotated by a quarter turn about z (left→left,
//! posterior→−x), which is a proper rotation: it preserves handedness and does
//! not mirror the brain. There is a test for exactly that, because a mirrored
//! brain still looks like a brain.
//!
//! Between the two sits **brain units**, which exist so that the connectome's
//! own numbers stay human-sized (the brain is ~20 units long, not 0.8). One
//! brain unit is 40.675 µm.

/// Raw nanometre offset subtracted before scaling: the fitted centre of the
/// FAFB v783 volume. Chosen so the brain is centred on the origin.
pub const CENTER_NM: [f64; 3] = [494_398.0, 247_130.0, 139_887.0];

/// Brain units per nanometre.
const UNITS_PER_NM: f64 = 2.458_52e-05;

/// One brain unit, in millimetres. Equivalently 40.675 µm.
pub const MM_PER_BRAIN_UNIT: f32 = 0.040_675;

/// FlyWire nanometres → brain units, reorienting into the world frame.
///
/// The permutation is the whole point: raw is (left, posterior, dorsal) and
/// world is (anterior, left, dorsal), so raw y becomes −world x and raw x
/// becomes world y.
#[inline]
pub fn raw_to_brain(raw: [f64; 3]) -> [f32; 3] {
    let dx = raw[0] - CENTER_NM[0]; // + = animal left
    let dy = raw[1] - CENTER_NM[1]; // + = posterior
    let dz = raw[2] - CENTER_NM[2]; // + = dorsal
    [
        (-dy * UNITS_PER_NM) as f32, // anterior
        (dx * UNITS_PER_NM) as f32,  // animal left
        (dz * UNITS_PER_NM) as f32,  // dorsal
    ]
}

/// Brain units → world millimetres.
#[inline]
pub fn brain_to_mm(brain: [f32; 3]) -> [f32; 3] {
    [
        brain[0] * MM_PER_BRAIN_UNIT,
        brain[1] * MM_PER_BRAIN_UNIT,
        brain[2] * MM_PER_BRAIN_UNIT,
    ]
}

/// FlyWire nanometres → world millimetres, for the mesh converter.
#[inline]
pub fn raw_to_mm(raw: [f64; 3]) -> [f32; 3] {
    brain_to_mm(raw_to_brain(raw))
}

/// A span measured in raw nanometres, expressed in millimetres. Goes through
/// the same constants as everything else so the scale is written down once.
#[inline]
pub const fn raw_span_to_mm(span_nm: f64) -> f64 {
    span_nm * UNITS_PER_NM * MM_PER_BRAIN_UNIT as f64
}

/// The FAFB v783 bounding box in raw nanometres, measured from
/// `coordinates.csv`. `etl` refuses to run if the coordinate file does not
/// span this, because a truncated download would otherwise produce a
/// confidently wrong brain.
pub const EXPECTED_BBOX_NM: [[f64; 3]; 2] = [
    [87_624.0, 51_360.0, 640.0],
    [902_880.0, 442_888.0, 279_120.0],
];

/// Published extent of the FAFB brain, in millimetres. The transform has to
/// reproduce this or the scale constant is wrong.
pub const BRAIN_MM: [f64; 3] = [
    raw_span_to_mm(EXPECTED_BBOX_NM[1][1] - EXPECTED_BBOX_NM[0][1]), // raw y → world x
    raw_span_to_mm(EXPECTED_BBOX_NM[1][0] - EXPECTED_BBOX_NM[0][0]), // raw x → world y
    raw_span_to_mm(EXPECTED_BBOX_NM[1][2] - EXPECTED_BBOX_NM[0][2]), // raw z → world z
];

// --- anatomy, for the body the brain has to fit inside ----------------------

/// Length of an adult Drosophila, nose to tail, in millimetres.
pub const FLY_LENGTH_MM: f32 = 2.5;

/// Where the brain sits inside that body: the head is the front ~20%, and the
/// brain fills most of it. Numbers from the usual adult measurements — head
/// capsule ~0.5 mm long, the brain ~0.8 mm across its widest (the optic lobes)
/// and ~0.4 mm along the body axis.
pub const HEAD_LENGTH_MM: f32 = 0.5;

/// World scale: the brain mesh arrives in millimetres already, so the world is
/// in millimetres and no extra factor is needed. This constant exists to make
/// that explicit rather than implicit.
pub const WORLD_UNITS_PER_MM: f32 = 1.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    #[test]
    fn the_centre_maps_to_the_origin() {
        let z = raw_to_brain(CENTER_NM);
        assert!(z.iter().all(|v| close(*v, 0.0, 1e-6)), "{z:?}");
    }

    #[test]
    fn one_micron_is_the_expected_step() {
        let p = raw_to_brain([CENTER_NM[0] + 1000.0, CENTER_NM[1], CENTER_NM[2]]);
        assert!(close(p[1], 0.024_585_2, 1e-6), "raw x should become world y: {p:?}");
        assert!(close(p[1] * MM_PER_BRAIN_UNIT, 0.001, 1e-6), "{p:?}");
    }

    #[test]
    fn the_axes_land_where_anatomy_says_they_should() {
        // A neuron in front of centre: low raw y.
        let front = raw_to_brain([CENTER_NM[0], CENTER_NM[1] - 1000.0, CENTER_NM[2]]);
        assert!(front[0] > 0.0, "raw −y must become world +x (anterior): {front:?}");
        // A neuron on the animal's left: high raw x.
        let left = raw_to_brain([CENTER_NM[0] + 1000.0, CENTER_NM[1], CENTER_NM[2]]);
        assert!(left[1] > 0.0, "raw +x must become world +y (left): {left:?}");
        // A dorsal neuron: high raw z.
        let up = raw_to_brain([CENTER_NM[0], CENTER_NM[1], CENTER_NM[2] + 1000.0]);
        assert!(up[2] > 0.0, "raw +z must become world +z (dorsal): {up:?}");
    }

    #[test]
    fn the_transform_is_a_rotation_not_a_mirror() {
        // A mirrored brain still looks like a brain, so this needs an actual
        // check: a right-handed triple must stay right-handed. Take the raw
        // frame's own basis (left, posterior, dorsal) — right-handed by
        // definition — and confirm the cross product still points the same way
        // after mapping.
        let o = CENTER_NM;
        let d = 100_000.0; // 100 µm, large enough to clear f32 noise
        let px = raw_to_brain([o[0] + d, o[1], o[2]]); // animal left
        let py = raw_to_brain([o[0], o[1] + d, o[2]]); // posterior
        let pz = raw_to_brain([o[0], o[1], o[2] + d]); // dorsal

        let cross = [
            px[1] * py[2] - px[2] * py[1],
            px[2] * py[0] - px[0] * py[2],
            px[0] * py[1] - px[1] * py[0],
        ];
        // Roughly along +pz means the handedness survived.
        let dot = cross[0] * pz[0] + cross[1] * pz[1] + cross[2] * pz[2];
        assert!(dot > 0.0, "the transform mirrors the brain (det = −1): {cross:?}");
    }

    #[test]
    fn the_whole_brain_lands_at_the_size_a_brain_should_be() {
        // The brain is widest left-to-right (the optic lobes), which after
        // reorientation is the world y axis.
        assert!(close(BRAIN_MM[1] as f32, 0.813, 0.01), "width {}", BRAIN_MM[1]);
        assert!(close(BRAIN_MM[0] as f32, 0.391, 0.01), "length {}", BRAIN_MM[0]);
        assert!(close(BRAIN_MM[2] as f32, 0.278, 0.01), "height {}", BRAIN_MM[2]);
        // And the long axis is the left-right one, which is what a fly brain
        // with its optic lobes actually looks like.
        assert!(BRAIN_MM[1] > BRAIN_MM[0], "optic lobes should make the brain widest");
    }

    #[test]
    fn the_brain_fits_in_the_head() {
        let body = FLY_LENGTH_MM;
        assert!(BRAIN_MM[0] as f32 <= HEAD_LENGTH_MM, "brain is longer than the head");
        assert!(BRAIN_MM[0] as f32 > body * 0.1, "brain is implausibly small");
    }
}
