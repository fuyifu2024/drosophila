//! Reading the brain's output, and steering with it.
//!
//! This is the most dishonest file in the project, and it is worth being
//! precise about how.
//!
//! The brain's output port is real: 1,305 descending neurons plus 110 motor
//! neurons, grouped by side, firing rates read straight out of the simulation.
//! What is *not* real is the map from those rates to a control stick. A fly
//! does not have a "yaw channel"; its descending neurons carry specific
//! commands to specific muscles, and the mapping from spike rate to wing
//! kinematics is a research programme, not a function in a struct.
//!
//! So the arrangement here is:
//!
//! * **A hard-wired reflex does the flying.** Seek the light, avoid the walls,
//!   hold an altitude. This is what keeps the fly in the air and makes it look
//!   like something an animal would do. It is not the brain, and it would work
//!   just as well with the connectome unplugged.
//!
//! * **The brain perturbs it.** Descending-neuron activity, high-passed,
//!   nudges yaw and thrust. This is genuinely the connectome — the fly's
//!   moment-to-moment twitches, its wandering, its occasional lurch come from
//!   139,255 neurons firing.
//!
//! The high-pass is not a detail. The connectome has a *persistent* left–right
//! asymmetry (more neurons, more synapses, on one side than the other in any
//! sampled subnetwork), so a raw left-minus-right readout is a constant offset,
//! and the fly would turn steadily in one direction forever. Subtracting an
//! 8-second running mean leaves only the fluctuations, which is what actually
//! carries information. The same trick is in gnat, for the same reason.

use crate::body::{Body, BodyInput, Flight, CRUISE_SPEED};
use crate::lif::Sim;
use crate::senses::Senses;
use crate::wiring::Wiring;
use crate::world::World;

/// Altitude the reflex tries to hold, in mm — mid-height, so there is room to
/// climb or dive in either direction.
const TARGET_Z: f32 = crate::world::HEIGHT * 0.5;

/// Altitude-hold gains: rad/s of pitch command per mm of altitude error, and
/// per mm/s of climb rate. See the derivation at their use site — the second
/// one is not a refinement, it is the difference between a stable loop and an
/// unstable one.
const K_ALT_P: f32 = 0.016;
const K_ALT_D: f32 = 0.016;

/// Range at which the walls start pushing back, mm.
///
/// This number is not a taste decision, it is set by the physics. At cruise the
/// fly covers `CRUISE_SPEED` millimetres per second and can yaw at about 4
/// radians per second, so its tightest turn has a radius of roughly
/// `CRUISE_SPEED / 4` — about 38 mm. Any avoidance range smaller than that is
/// decorative: the fly cannot physically turn in time, and the first tuning
/// pass with a 25 mm range produced 2,141 collisions in twenty seconds.
const AVOID_RANGE: f32 = 45.0;
const AVOID_STRENGTH: f32 = 10.0;

/// Thrust multiplier when right up against a surface. Slowing down shortens the
/// turn radius quadratically, which is how the fly gets out of a corner it has
/// already committed to.
const CORNER_THRUST: f32 = 0.35;

/// How close to the light, in mm, the fly decides it has arrived and lands.
///
/// Landing is what turns "a fly that circles a lamp" into "a fly". It is also
/// the only way walking ever happens, so this number controls how much of the
/// time the animal is on its feet.
const LANDING_RANGE: f32 = 22.0;

/// How long a flight has to last before the fly will consider landing.
///
/// Two earlier versions of the landing rule got this wrong in opposite
/// directions, and both were measured:
///
/// * **No minimum.** It landed the moment it came within range — including
///   immediately after taking off from beside the lamp. The steady state was
///   22 seconds on the ground to 1.5 in the air: 92% of the run on its feet,
///   and its "flights" were hops.
/// * **An excursion requirement.** It had to have been 44 mm from the lamp
///   since taking off. That fixed the hop and broke the other end: a fly that
///   orbits the lamp at 25 mm never satisfies it, so it flew for the entire
///   fifty-second run and never landed once.
///
/// A minimum flight time is the honest version of both. Fly a while; land when
/// you arrive.
const MIN_FLIGHT_SECONDS: f32 = 8.0;

/// Radial closing speed below which an arrival does not count, in mm/s. Not
/// zero, because a fly that happens to be drifting outward at the moment it
/// crosses the range is on its way somewhere else.
const CLOSING_SPEED: f32 = -10.0;

/// Thrust that balances drag at cruise.
const BASE_THRUST: f32 = CRUISE_SPEED / crate::body::DRAG_TAU;

pub struct Motor {
    /// Seconds. The high-pass time constant on the descending readout.
    pub tau: f32,
    /// How hard brain activity is allowed to push the controls. Zero gives a
    /// pure reflex fly, which is worth being able to see for comparison.
    pub brain_yaw: f32,
    pub brain_thrust: f32,
    /// How much say the hard-wired reflex has. Below 1 the fly gets clumsy.
    pub reflex: f32,
    /// Running means, per side, for the high-pass.
    mean_left: f32,
    mean_right: f32,
    /// Once the fly has decided to land, it stays decided.
    ///
    /// Without the latch the descent flickered and it never touched the
    /// ground: it would commit, start down, drift back out of range on its
    /// orbit, abort, climb, commit again. The trace showed altitude sagging
    /// from 60 to 50 and springing back, over and over, for a hundred and
    /// twenty seconds. A decision that the next millisecond's geometry can
    /// cancel is not a decision.
    landing_committed: bool,
    /// Where the altitude loop is currently aiming, ramped rather than
    /// stepped.
    alt_target: f32,
    pub state: MotorState,
}

/// How fast the altitude *target* is allowed to move, in mm/s.
///
/// Stepping it from cruise to the floor in one tick is a 60 mm step input, and
/// on a loop whose damping term is proportional to climb rate that means the
/// damping wins: measured, the fly porpoised between 14 and 38 mm for sixteen
/// seconds and never once reached the ground. Ramping the target turns the
/// descent into something the loop can track — a controlled glide down at
/// roughly this speed.
const DESCENT_RATE: f32 = 45.0;

/// The last thing the motor layer did, for the report and the viewer.
#[derive(Clone, Copy, Debug, Default)]
pub struct MotorState {
    pub dn_left_hz: f32,
    pub dn_right_hz: f32,
    /// Left and right after the high-pass — what actually reaches the stick.
    pub dn_turn: f32,
    pub dn_thrust: f32,
    pub yaw_cmd: f32,
    pub pitch_cmd: f32,
    pub thrust: f32,
    /// Horizontal distance to the light, mm. Above `LANDING_RANGE` the fly is
    /// travelling; below it, it is arriving.
    pub dist_to_light: f32,
    /// Whether the reflex asked to land this tick.
    pub landing: bool,
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / n, v[1] / n, v[2] / n]
}

impl Default for Motor {
    fn default() -> Self {
        Motor {
            tau: 8.0,
            brain_yaw: 12.0,
            brain_thrust: 900.0,
            reflex: 1.0,
            mean_left: 0.0,
            mean_right: 0.0,
            landing_committed: false,
            alt_target: TARGET_Z,
            state: MotorState::default(),
        }
    }
}

impl Motor {
    /// One millisecond. Returns what to tell the body: the control inputs, and
    /// whether this is a good moment to be on the ground.
    pub fn read(
        &mut self,
        sim: &Sim,
        w: &Wiring,
        b: &Body,
        world: &World,
        senses: &Senses,
        dt: f32,
    ) -> (Flight, BodyInput) {
        // --- what the brain is saying ---
        let dn_left = sim.group_hz(&w.descending_left);
        let dn_right = sim.group_hz(&w.descending_right);
        let a = 1.0 - (-dt / self.tau).exp();
        self.mean_left += (dn_left - self.mean_left) * a;
        self.mean_right += (dn_right - self.mean_right) * a;
        let turn = (dn_left - self.mean_left) - (dn_right - self.mean_right);
        let drive = (dn_left - self.mean_left) + (dn_right - self.mean_right);

        // --- the reflex: where should it be pointing? ---
        let to_light = world.towards_light(b.pos);
        let (gap, n) = world.nearest_surface(b.pos);
        // Horizontal distance to the lamp. Only horizontal: a fly lands beside
        // the light, not on it, and the light is a point 95 mm up.
        let dx = world.light[0] - b.pos[0];
        let dy = world.light[1] - b.pos[1];
        let dist = (dx * dx + dy * dy).sqrt();
        let closing = (b.vel[0] * dx + b.vel[1] * dy) / dist.max(1e-3);

        // Landing is a commitment, not a per-tick test.
        //
        // The condition below says "arrived". Once it holds, the fly keeps
        // going down until it is on the ground or has clearly been blown off
        // course; it does not re-evaluate every millisecond, because on an
        // orbit its distance from the lamp is always crossing the threshold in
        // both directions.
        if b.is_grounded() {
            self.landing_committed = false;
        } else if !self.landing_committed
            && b.gait_time > MIN_FLIGHT_SECONDS
            && dist < LANDING_RANGE
            && closing > CLOSING_SPEED
        {
            self.landing_committed = true;
        }
        // Two ways to give up: knocked well off course, or taking absurdly
        // long about it for some reason the model did not anticipate.
        if self.landing_committed
            && (dist > LANDING_RANGE * 2.5 || b.gait_time > MIN_FLIGHT_SECONDS + 25.0)
        {
            self.landing_committed = false;
        }
        let landing = !b.is_grounded() && self.landing_committed;
        // Walls push harder the closer they are, and the push is along the
        // surface normal, which points back into open space.
        let closeness = ((AVOID_RANGE - gap) / AVOID_RANGE).clamp(0.0, 1.0);

        // But only push off a surface the fly is actually *heading into*.
        //
        // A pure potential field repels from everything in range regardless of
        // direction, and in a room this size that means the lamp is
        // unreachable: the corridor leading to it passes within the avoidance
        // range of the wall behind it, so the repulsion cancels the attraction
        // and the fly circles the middle of the room forever. Which is exactly
        // what it did — the light was doing nothing, and the fly looked like it
        // was wandering.
        // A flying fly is going to the light. A fly on its feet is going to the
        // nearest food — it already got where the light was, and on foot it
        // wants something to eat.
        //
        // `is_grounded`, not `is_walking`. With `is_walking` a *grooming* fly
        // fell through to the light branch, so `goal_dist` was its distance to
        // the lamp, 40–60 mm, which is never within `FOOD_RADIUS` — and
        // `at_food` below was therefore permanently false while it stood
        // exactly on the food. Nothing read `at_food` in that state yet, so it
        // did no visible harm; it just meant the flag was lying.
        let (goal_dist, goal) = if b.is_grounded() {
            world.nearest_food(b.pos)
        } else {
            (dist, to_light)
        };

        let f = b.forward();
        let into = -(f[0] * n[0] + f[1] * n[1]).clamp(-1.0, 1.0);
        let urgency = closeness * into.max(0.0);
        let repulsion = urgency * AVOID_STRENGTH;
        let mut desire = norm([
            goal[0] + n[0] * repulsion,
            goal[1] + n[1] * repulsion,
            goal[2] + n[2] * repulsion * 0.7,
        ]);

        // And when it is close to something, slide along it rather than press
        // into it: pick the tangent that leads towards the light and lean on
        // that. A fly that only ever gets pushed straight out of a wall sits
        // against it and grinds, which is 5,283 collisions in ninety seconds.
        if closeness > 0.25 {
            let tangent = [-n[1], n[0], 0.0];
            let toward = tangent[0] * goal[0] + tangent[1] * goal[1];
            let side = if toward >= 0.0 { 1.0 } else { -1.0 };
            let push = closeness * 3.0;
            desire = norm([
                desire[0] + tangent[0] * side * push,
                desire[1] + tangent[1] * side * push,
                desire[2],
            ]);
        }

        // Heading error in the horizontal plane only; altitude is the pitch
        // channel's job.
        let cross = f[0] * desire[1] - f[1] * desire[0];
        let dot = f[0] * desire[0] + f[1] * desire[1];
        let yaw_err = cross.atan2(dot);

        // A walking fly turns far more slowly than a flying one. At 28 mm/s the
        // flying turn rate of 4 rad/s is a radius of seven millimetres — the
        // fly spins on the spot and never goes anywhere, which is exactly what
        // the first version of this did.
        let mut yaw_cmd = yaw_err * if b.is_walking() { 0.6 } else { 3.5 };

        // Altitude hold, as a *rate* command on the pitch, and it has to have
        // the rate term or the loop is unstable.
        //
        // Pitch is a pure integrator, so the loop is: altitude error → pitch
        // rate → pitch → vertical acceleration → altitude. Three integrations
        // around a negative feedback. Routh–Hurwitz on the characteristic
        // polynomial `s³ + (1/τ)s² + 0·s + T·k` needs `a·b > c`, and here
        // `b = 0`, so `a·b − c = −T·k < 0` — **unstable for every value of
        // thrust**, with a pole pair at `+0.10 ± 1.32i`: a 4.8-second
        // oscillation growing 11% per second.
        //
        // That was not theory. The fly did not cruise at 60 mm; it bounced
        // between the floor clamp and the ceiling for the entire first night,
        // and its own z column said so (`68 → 34 → 9.7 → 0.6 → 46.8`, plus a
        // recurring `118.8`) while nobody read it.
        //
        // `− vel[2] * K_ALT_D` puts the `s` back: the polynomial becomes
        // `s³ + (1/τ)s² + T·K_D·s + T·K_P`, and `a·b/c = (K_D/K_P)/τ` — stable
        // for any thrust as long as the gains are within a factor of about
        // 8000 of each other. Equal gains give a margin of 8.3.
        // Aim at the floor once committed, but walk the aim point there.
        let goal = if landing {
            crate::body::STAND_HEIGHT
        } else {
            TARGET_Z
        };
        let step = DESCENT_RATE * dt;
        self.alt_target = if self.alt_target < goal {
            (self.alt_target + step).min(goal)
        } else {
            (self.alt_target - step).max(goal)
        };
        let mut pitch_cmd = (self.alt_target - b.pos[2]) * K_ALT_P - b.vel[2] * K_ALT_D;
        pitch_cmd = pitch_cmd.clamp(-1.6, 1.6);
        // Climbing out of a corner needs more than altitude hold — *unless*
        // the surface in question is the one being landed on.
        //
        // This was the thing actually fighting the descent. At 15 mm up, the
        // proximity term alone commands a full-rate climb (+1.6 rad/s, which
        // saturates the pitch limit), and the fly porpoised between 14 and
        // 38 mm for sixteen seconds without ever touching the ground — while
        // the altitude loop was dutifully aiming lower the whole time. Landing
        // means the floor is the destination, not an obstacle.
        let avoid = if landing { 0.0 } else { 1.0 };
        if gap < AVOID_RANGE {
            pitch_cmd += n[2] * closeness * 1.6 * avoid;
        }

        yaw_cmd *= self.reflex;
        pitch_cmd *= self.reflex;

        // --- the brain nudges it ---
        yaw_cmd += turn * self.brain_yaw;
        // The altitude term only means anything in the air. Left switched on
        // while walking it adds a constant 356 mm/s² — nearly a third of the
        // total — and the fly walks at 36 mm/s instead of the 28 that
        // `WALK_SPEED` advertises, which makes that knob a lie.
        let climb_boost = if b.is_grounded() {
            0.0
        } else {
            (TARGET_Z - b.pos[2]).max(0.0) * 6.0
        };
        let mut thrust = BASE_THRUST + drive * self.brain_thrust + climb_boost;
        // Ease off near a surface. Slower means a tighter turn, and a tighter
        // turn means the repulsion above actually gets a chance to work.
        // Two reasons to ease off, and they multiply, so they are combined into
        // one scale that is then floored.
        //
        // Near a surface, slower means a tighter turn. On the approach, a fly
        // does not arrive at a landing site at cruise speed — and without the
        // slowdown it cannot arrive at all, because landing takes a minimum of
        // 1.5 s of flight and 1.5 s at cruise is 225 mm, wider than the room.
        let arriving = ((LANDING_RANGE * 1.8 - dist) / (LANDING_RANGE * 1.8)).clamp(0.0, 1.0);
        let corner = CORNER_THRUST + (1.0 - CORNER_THRUST) * (1.0 - closeness);
        let approach = 1.0 - 0.78 * arriving;
        // 0.35 × 0.22 is 8% of nominal, which is not enough to fly out of a
        // corner the fly has already got itself into. A fifth is.
        thrust *= (corner * approach).max(0.20);

        // Stop on the food. A fly that walks up to something to eat and then
        // walks straight over it is not eating; it is a Roomba.
        if b.is_walking() && goal_dist < crate::world::FOOD_RADIUS {
            thrust *= 0.04;
        }

        // A looming surface overrides everything else — but *along its normal*,
        // not upwards. Getting this wrong is subtle and was: an unconditional
        // climb, added on top of the normal-based term above, meant that a fly
        // pinned against the ceiling saw a maximum loom signal and responded by
        // climbing harder. It stayed there for the rest of the run.
        let alarm = (senses.state.loom - 1.5).max(0.0);
        let pitch_cmd = pitch_cmd + n[2] * alarm * 1.2 * avoid;

        // --- is this a good place to stop? (decided above, with the altitude) ---

        // Report what is actually sent to the body, not the raw command — a
        // HUD reading +9.2 rad/s next to a fly that is clearly not spinning at
        // +9.2 rad/s is worse than no HUD.
        let flight = Flight {
            thrust: thrust.clamp(0.0, 6000.0),
            yaw: yaw_cmd.clamp(-4.0, 4.0),
            pitch: pitch_cmd.clamp(-2.0, 2.0),
            roll: 0.0, // the body banks into the turn on its own
        };

        self.state = MotorState {
            dn_left_hz: dn_left,
            dn_right_hz: dn_right,
            dn_turn: turn,
            dn_thrust: drive,
            yaw_cmd: flight.yaw,
            pitch_cmd: flight.pitch,
            thrust: flight.thrust,
            dist_to_light: dist,
            landing,
        };
        // The raw loom, not the shaped `alarm` above: taking off is a threshold
        // response, and it is the same number the body uses to decide.
        //
        // `is_grounded` rather than `is_walking`, so a fly that is already
        // grooming still counts as standing on its food. Reading `is_walking`
        // here would drop `at_food` the instant grooming began, and the fly
        // would flicker between the two states.
        let at_food = b.is_grounded() && goal_dist < crate::world::FOOD_RADIUS;
        (
            flight,
            BodyInput {
                land: landing,
                startle: senses.state.loom,
                at_food,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lif::Params;

    /// The smallest circuit `Wiring` will accept: two neurons, one per side,
    /// both in the descending group, no edges. The brain is silent, so
    /// anything that happens is the reflex's doing.
    fn bare() -> (crate::circuit::Circuit, Wiring) {
        let mut b = Vec::new();
        b.extend_from_slice(&crate::etl::MAGIC.to_le_bytes());
        b.extend_from_slice(&crate::etl::VERSION.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        for i in 0..2u64 {
            b.extend_from_slice(&i.to_le_bytes());
            for v in [0.0f32, if i == 0 { 5.0 } else { -5.0 }, 0.0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[0, i as u8, 0, 1]);
            b.extend_from_slice(&[crate::etl::G_DESCENDING, 0, 0, 0]);
        }
        for _ in 0..3 {
            b.extend_from_slice(&0u32.to_le_bytes());
        }
        let c = crate::circuit::Circuit::from_bytes(&b).unwrap();
        let w = Wiring::new(&c);
        (c, w)
    }

    /// The altitude loop, closed.
    ///
    /// This is the test whose absence let an unstable controller ship. Every
    /// test in `body.rs` passes `Flight { pitch: 0, .. }`, so the one signal
    /// the fly actually uses to hold an altitude — the pitch rate the motor
    /// computes from the altitude error — was never exercised by anything.
    /// The fly bounced between the floor and the ceiling for a whole night
    /// with a green test suite.
    #[test]
    fn the_fly_holds_an_altitude_instead_of_bouncing_between_the_floor_and_the_ceiling() {
        let (c, w) = bare();
        let sim = Sim::new(&c, Params::default(), 1);
        let world = World::new();
        let senses = Senses::default();
        let mut motor = Motor::default();

        // Start level and 15 mm low, so the loop has a real error to work on.
        // Starting *at* the target gives the unstable mode nothing to grow
        // from, and the first version of this test did that.
        let mut body = Body::new([-40.0, -20.0, TARGET_Z - 15.0]);
        body.vel = [0.0, 0.0, 0.0];

        // Ten seconds, sampling the altitude error.
        //
        // The horizontal degrees of freedom are pinned and landing is
        // suppressed, because without that the test does not test the altitude
        // loop at all — and a first attempt at it, which did neither, passed
        // with the damping term removed. The fly reached the lamp in under a
        // second, landed, and once it is walking the pitch channel is ignored
        // entirely. The instability was real but it was over before the
        // assertion ran. Wall bounces would be the same kind of noise.
        let mut samples = Vec::new();
        for _ in 0..20_000 {
            let (flight, mut input) = motor.read(&sim, &w, &body, &world, &senses, 0.001);
            input.land = false;
            body.step(flight, &world, 0.001, input);
            body.pos[0] = -40.0;
            body.pos[1] = -20.0;
            body.vel[0] = 0.0;
            body.vel[1] = 0.0;
            samples.push((body.pos[2] - TARGET_Z).abs());
        }

        let window = |a: usize, b: usize| samples[a..b].iter().cloned().fold(0.0f32, f32::max);

        // The band it settles into, over the last second. A stable loop driven
        // from a 15 mm error should be back on target and quiet by then.
        let settled = window(samples.len() - 1000, samples.len());
        assert!(
            settled < 6.0,
            "the fly should settle within 6 mm of {TARGET_Z} mm, but its last second \
             wandered {settled:.1} mm"
        );

        // And the actual failure mode: an error that is *growing*. Measured at
        // 11% per second, so over the fifteen seconds between these two windows
        // the unstable version is several times louder than the stable one,
        // whatever the absolute numbers happen to be.
        let early = window(4_000, 5_000);
        let late = window(16_000, 17_000);
        assert!(
            late <= early.max(2.0),
            "the altitude error is growing: {early:.2} mm at t=4–5 s, {late:.2} mm at \
             t=16–17 s — the loop is unstable"
        );
    }

    #[test]
    fn with_no_brain_input_the_reflex_still_steers_towards_the_light() {
        // A circuit with no descending neurons at all: the brain is silent and
        // the reflex alone has to fly the fly.
        let (c, w) = bare();
        let sim = Sim::new(&c, Params::default(), 1);
        let world = World::new();
        let senses = Senses::default();
        let mut motor = Motor::default();

        // Start facing away from the light so there is a real error to correct.
        let mut body = Body::new(world.start_position());
        let to = world.towards_light(body.pos);
        body.yaw = to[1].atan2(to[0]) + std::f32::consts::PI;
        let before = (body.yaw - to[1].atan2(to[0])).abs();

        // One second of flight under reflex alone. The light is 145 mm away, so
        // nothing here should trigger a landing.
        for _ in 0..1000 {
            let (flight, input) = motor.read(&sim, &w, &body, &world, &senses, 0.001);
            body.step(flight, &world, 0.001, input);
        }
        assert!(!body.is_walking(), "should not have landed this far out");
        let after = {
            let t = world.towards_light(body.pos);
            let want = t[1].atan2(t[0]);
            let mut e = body.yaw - want;
            while e > std::f32::consts::PI {
                e -= std::f32::consts::TAU;
            }
            while e < -std::f32::consts::PI {
                e += std::f32::consts::TAU;
            }
            e.abs()
        };
        assert!(
            after < before * 0.6,
            "the reflex should have turned towards the light: {before} → {after}"
        );
    }
}
