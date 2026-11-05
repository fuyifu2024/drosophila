//! The world, as current injected into real sensory neurons.
//!
//! Everything here works the same way: compute one number describing some
//! property of the world, then spread that number as current across a named
//! population of labelled neurons. The retina does not get a picture; it gets
//! a scalar. Nothing downstream is told what the scalar means.
//!
//! Two approximations are worth naming, because both are bigger than they
//! look:
//!
//! * **The eye is a point.** Real compound eyes have ~800 ommatidia sampling a
//!   nearly hemispheric field. Here each eye is one direction vector, so the
//!   fly has no spatial vision at all — only "how bright is my left" and "how
//!   bright is my right".
//! * **The directional labels on T4 and T5 are assumed, not verified.** The
//!   four subtypes a–d are the four cardinal directions of visual motion, but
//!   which is which was not checked against the literature here. a and b are
//!   mapped to the fore–aft axis and c and d to the vertical one. If the
//!   assignment is backwards, the fly will steer away from the light instead
//!   of towards it, which is at least a visible failure.

use crate::body::Body;
use crate::lif::Sim;
use crate::wiring::Wiring;
use crate::world::World;

/// Total current injected, spread across each population. Small on purpose:
/// the membrane integrates at 1 kHz, so a steady 0.004 per tick is a steady
/// state of about 0.08 against a threshold of 1.0 — enough to bias the firing
/// rate without driving the whole brain into saturation.
#[derive(Clone, Copy, Debug)]
pub struct Gains {
    pub photo: f32,
    pub motion: f32,
    pub loom: f32,
}

impl Default for Gains {
    fn default() -> Self {
        Gains {
            photo: 0.010,
            motion: 0.006,
            loom: 0.020,
        }
    }
}

/// What the senses last told the brain. Kept so the run loop can print it and
/// the viewer can draw it — a fly whose input is stuck at zero looks identical
/// to a fly whose brain has died, and they are very different bugs.
#[derive(Clone, Copy, Debug, Default)]
pub struct SenseState {
    pub eye_left: f32,
    pub eye_right: f32,
    /// Image motion along the four axes: fore, aft, down, up.
    pub motion: [f32; 4],
    /// Closing speed divided by distance: how fast the nearest surface is
    /// expanding in the visual field.
    pub loom: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Senses {
    pub gains: Gains,
    pub state: SenseState,
}

impl Default for Senses {
    fn default() -> Self {
        Self::new(Gains::default())
    }
}

/// Spread `total` current evenly over a population. Working in totals rather
/// than per-neuron means a population of 11,000 photoreceptors and one of 210
/// looming detectors deliver comparable drive, which is what you want when the
/// populations differ in size by fifty times for reasons that have nothing to
/// do with how much the fly cares.
fn drive(sim: &mut Sim, group: &[u32], total: f32) {
    if group.is_empty() || total == 0.0 {
        return;
    }
    let per = total / group.len() as f32;
    for &i in group {
        sim.stimulate(i as usize, per);
    }
}

/// Outward normal of one eye, as a blend of forward and sideways. A real
/// compound eye faces most of the way sideways, and this puts the two eye
/// normals about 100° apart, which is the right ballpark.
fn eye_normal(b: &Body, side: f32) -> [f32; 3] {
    let f = b.forward();
    let l = b.left();
    let v = [f[0] * 0.45 + l[0] * side, f[1] * 0.45 + l[1] * side, f[2] * 0.45 + l[2] * side];
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / n, v[1] / n, v[2] / n]
}

impl Senses {
    pub fn new(gains: Gains) -> Self {
        Senses {
            gains,
            state: SenseState::default(),
        }
    }

    /// Drive the sensory neurons for this millisecond.
    pub fn apply(&mut self, sim: &mut Sim, w: &Wiring, b: &Body, world: &World) {
        let to_light = world.towards_light(b.pos);
        let intensity = world.light_intensity(b.pos);

        // --- photoreceptors: how much light each eye is pointing at ---
        let nl = eye_normal(b, 1.0);
        let nr = eye_normal(b, -1.0);
        let dot = |a: [f32; 3], c: [f32; 3]| a[0] * c[0] + a[1] * c[1] + a[2] * c[2];
        let eye_left = intensity * dot(nl, to_light).max(0.0);
        let eye_right = intensity * dot(nr, to_light).max(0.0);
        self.state.eye_left = eye_left;
        self.state.eye_right = eye_right;
        drive(sim, &w.photoreceptor_left, self.gains.photo * eye_left);
        drive(sim, &w.photoreceptor_right, self.gains.photo * eye_right);

        // --- motion detectors: image velocity in the body's own frame ---
        // Self-motion forward makes the image flow backwards, so the sign here
        // is inverted relative to the body's velocity.
        let f = b.forward();
        let l = b.left();
        let u = b.up();
        let speed = 250.0; // mm/s that counts as "full scale" for a motion channel
        let fore = (dot(b.vel, f) / speed).clamp(-1.0, 1.0);
        let side = (dot(b.vel, l) / speed).clamp(-1.0, 1.0);
        let vert = (dot(b.vel, u) / speed).clamp(-1.0, 1.0);

        // a: image moving aft, b: image moving fore, c: down, d: up.
        // The sideways component is folded into the fore/aft pair, because a
        // yawing fly sees its whole field sweep horizontally and the horizontal
        // channels are the only place for that to arrive.
        let m = [
            (-fore).max(0.0) + (-side).max(0.0),
            fore.max(0.0) + side.max(0.0),
            (-vert).max(0.0),
            vert.max(0.0),
        ];
        self.state.motion = m;
        for k in 0..4 {
            drive(sim, &w.t4[k], self.gains.motion * m[k]);
            drive(sim, &w.t5[k], self.gains.motion * m[k] * 0.7);
        }

        // --- looming: closing speed over distance ---
        let (gap, _) = world.nearest_surface(b.pos);
        let closing = {
            // How fast the gap is shrinking. Positive means approaching.
            let v = b.vel;
            let (_, n) = world.nearest_surface(b.pos);
            -(v[0] * n[0] + v[1] * n[1] + v[2] * n[2])
        };
        let loom = (closing.max(0.0) / gap.max(1.0)).clamp(0.0, 6.0);
        self.state.loom = loom;
        drive(sim, &w.looming, self.gains.loom * loom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::circuit::Circuit;
    use crate::etl;
    use crate::lif::Params;

    fn tiny_circuit() -> Circuit {
        let mut b = Vec::new();
        b.extend_from_slice(&etl::MAGIC.to_le_bytes());
        b.extend_from_slice(&etl::VERSION.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        for (i, g) in [(0u64, etl::G_PHOTORECEPTOR), (1, etl::G_PHOTORECEPTOR)] {
            b.extend_from_slice(&i.to_le_bytes());
            for v in [0.0f32, 0.0, 0.0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[0, if i == 0 { 0 } else { 1 }, 0, 1]);
            b.extend_from_slice(&[g, 0, 0, 0]);
        }
        for _ in 0..3 {
            b.extend_from_slice(&0u32.to_le_bytes());
        }
        Circuit::from_bytes(&b).unwrap()
    }

    #[test]
    fn the_wiring_finds_the_photoreceptors() {
        let c = tiny_circuit();
        let w = Wiring::new(&c);
        assert_eq!(w.photoreceptor_left.len(), 1);
        assert_eq!(w.photoreceptor_right.len(), 1);
    }

    #[test]
    fn looking_straight_at_the_light_is_brighter_than_looking_away() {
        let c = tiny_circuit();
        let w = Wiring::new(&c);
        let world = World::new();
        let mut sim = Sim::new(&c, Params::default(), 1);
        let mut s = Senses::default();

        // Point the fly at the light.
        let mut b = Body::new(world.start_position());
        let to = world.towards_light(b.pos);
        b.yaw = to[1].atan2(to[0]);
        b.pitch = to[2].asin();

        s.apply(&mut sim, &w, &b, &world);
        let facing = s.state.eye_left + s.state.eye_right;

        // Now turn right around.
        b.yaw += std::f32::consts::PI;
        s.apply(&mut sim, &w, &b, &world);
        let away = s.state.eye_left + s.state.eye_right;

        assert!(
            facing > away * 1.5,
            "facing the light should be clearly brighter: {facing} vs {away}"
        );
    }

    #[test]
    fn the_eyes_disagree_when_the_light_is_to_one_side() {
        let c = tiny_circuit();
        let w = Wiring::new(&c);
        let world = World::new();
        let mut sim = Sim::new(&c, Params::default(), 1);
        let mut s = Senses::default();
        // Stand off the light's meridian, facing +x. The light is then ahead
        // and off to one side, which is the case the eyes have to disagree on.
        // (Sitting exactly on the meridian makes them agree, correctly.)
        let mut b = Body::new([-30.0, 25.0, 50.0]);
        b.yaw = 0.0;
        s.apply(&mut sim, &w, &b, &world);
        let diff = s.state.eye_left - s.state.eye_right;
        assert!(
            diff.abs() > 1e-3,
            "a side-mounted light must make the eyes differ: {:?}",
            s.state
        );
        // The light sits at y = 0 and the fly at y = +25, so it is off to the
        // animal's right and the right eye must be the brighter one.
        assert!(diff < 0.0, "expected the right eye to be brighter, got {diff}");
    }

    #[test]
    fn flying_at_a_wall_raises_the_loom_signal() {
        let c = tiny_circuit();
        let w = Wiring::new(&c);
        let world = World::new();
        let mut sim = Sim::new(&c, Params::default(), 1);
        let mut s = Senses::default();

        // Far from any wall, drifting slowly.
        let mut b = Body::new([0.0, 0.0, 60.0]);
        b.vel = [10.0, 0.0, 0.0];
        s.apply(&mut sim, &w, &b, &world);
        let calm = s.state.loom;

        // Right up against a wall, closing fast.
        let mut b = Body::new([crate::world::HALF_XY - 3.0, 0.0, 60.0]);
        b.vel = [200.0, 0.0, 0.0];
        s.apply(&mut sim, &w, &b, &world);
        let alarm = s.state.loom;

        assert!(
            alarm > calm * 5.0,
            "loom should spike near a wall: {calm} vs {alarm}"
        );
    }
}
