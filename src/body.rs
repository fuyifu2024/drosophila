//! The thing the brain is installed in.
//!
//! This is not a biomechanical model and does not pretend to be one. It is a
//! point mass with an attitude, two gaits, and four control channels, and it
//! exists so that the connectome's output has somewhere to go. Real flight
//! aerodynamics, walking kinematics and muscle dynamics are all absent; what is
//! here is the smallest body that can be steered convincingly.
//!
//! # Two gaits
//!
//! A fruit fly does not pick one. It flies to somewhere worth being, walks
//! around once it is there, and takes off again — either when it has finished
//! or when something startles it. That alternation is most of what makes a
//! small insect look alive, and a fly that only ever flies looks like a
//! screensaver. So the body has both, and switches between them on the same
//! signals a real one uses: proximity to what it wants, and a looming threat.
//!
//! The walking is a **tripod gait** — front-left, middle-right and back-left
//! step together while the other three are planted, then they swap. It is what
//! insects actually do, and it is also the reason a six-legged walk looks
//! stable rather than like a scurry.
//!
//! # Wings
//!
//! A fruit fly beats its wings at about 200 Hz, which a 60 Hz display cannot
//! show — sampled at 60 Hz, a 200 Hz oscillation is indistinguishable from a
//! 20 Hz one. So the body keeps the true phase, and separately exposes a
//! **visual** phase at a rate the screen can actually render. Pretending to
//! show 200 Hz would just be a flicker.

use crate::world::World;

/// True wingbeat frequency. Never drawn — see the module comment.
pub const WINGBEAT_HZ: f32 = 200.0;
/// What the wings are animated at, so the eye sees a blur that moves rather
/// than a strobe.
pub const WING_VISUAL_HZ: f32 = 18.0;

/// Cruise speed, mm/s. A fruit fly manages several hundred; this is dialled
/// back so the camera can follow it through a 200 mm room.
pub const CRUISE_SPEED: f32 = 150.0;

/// Walking speed, mm/s. A fly on foot manages about 20–50, and it looks wrong
/// any faster.
pub const WALK_SPEED: f32 = 28.0;

/// Steps per second while walking.
pub const GAIT_HZ: f32 = 4.5;

/// Rubs per second while grooming. Flies are brisk about it.
pub const GROOM_HZ: f32 = 3.2;

/// Height of the body's centre above the ground while standing, in mm.
///
/// A fly stands tall on its legs — the body is carried well clear of the
/// ground, and the legs are long relative to a 2.5 mm body. This started at
/// 0.34, which is a crouch, and made the legs look like stubs.
pub const STAND_HEIGHT: f32 = 0.55;

/// How far the body's centre must stay from a *vertical* surface, in mm. About
/// half the fly's length.
///
/// This is deliberately not used for the floor. A first version used one
/// standoff in every direction, and since half a fly is 1.25 mm and a standing
/// fly's body is only 0.34 mm up, the collision code shoved it off the ground
/// the moment it landed and held it there — legs waving in the air, 1.25 mm
/// above the floor, forever.
const WALL_STANDOFF: f32 = 1.1;

/// The hard floor of the body's altitude, below the collision standoff on
/// purpose — see its use in `step_flying`.
const FLOOR_CLAMP: f32 = STAND_HEIGHT * 0.86;

/// Velocity damping time constant, seconds. A fly is dominated by drag, not
/// inertia, so its velocity follows its thrust almost immediately. Public
/// because the motor layer works out the thrust that balances drag from it,
/// and two copies of this number would drift apart.
pub const DRAG_TAU: f32 = 0.12;

/// The same, on six feet. Higher, because feet grip: a walking fly's velocity
/// tracks its drive almost exactly, and it stops when it stops pushing.
pub const WALK_TAU: f32 = 0.05;

/// Sink rate when not thrusting. Small, because drag does most of the work.
const GRAVITY: f32 = 90.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    Flying,
    Walking,
    /// Standing still and cleaning its face with its front legs.
    ///
    /// This is not decoration. A fly grooms constantly — it is one of the
    /// things a fly is *for* — and an animal that only ever travels between
    /// two behaviours never looks like it is doing anything for its own
    /// reasons. It costs one state and one animation.
    Grooming,
    /// Wings out, beating hard, legs coiled — the fraction of a second before
    /// it leaves the ground.
    ///
    /// Every route from the ground to the air goes through here, which is the
    /// point: a fly that jumps from a standstill looks thrown, and no amount
    /// of tuning the jump velocity fixes that, because what is missing is the
    /// part before the jump.
    Warming,
}

impl Gait {
    pub fn code(self) -> u32 {
        match self {
            Gait::Flying => 0,
            Gait::Walking => 1,
            Gait::Grooming => 2,
            Gait::Warming => 3,
        }
    }

    /// On the ground, in the sense that matters to the wings and the legs.
    pub fn is_grounded(self) -> bool {
        !matches!(self, Gait::Flying)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Flight {
    /// Commanded acceleration along the body's forward axis, mm/s².
    pub thrust: f32,
    /// Body rates, radians per second.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

/// What the rest of the world wants the body to do about being on the ground.
#[derive(Clone, Copy, Debug, Default)]
pub struct BodyInput {
    /// The reflex has decided this is a good place to stop.
    pub land: bool,
    /// How alarming the world currently is. Above a threshold, a walking fly
    /// takes off, which is the correct response and also the interesting one.
    pub startle: f32,
    /// Standing on something worth eating. A fly that has found food stops and
    /// cleans up, rather than walking over it and continuing.
    pub at_food: bool,
}

/// Startle level at which a walking fly leaves the ground.
pub const STARTLE_TAKEOFF: f32 = 2.0;

/// How long a fly will walk about on food before it stops to clean up.
///
/// This is also the gap *between* grooming bouts, because the timer restarts
/// every time it goes back on its feet. At 2.5 s it groomed, took two steps,
/// groomed again — a fidget. Six seconds gives it a walk in between, which is
/// what a fly actually does.
pub const FEED_BEFORE_GROOM: f32 = 6.0;

/// How long a grooming bout lasts.
pub const GROOM_SECONDS: f32 = 5.0;

/// Total time on the ground before it takes off anyway, in seconds.
pub const GROUND_BUDGET: f32 = 22.0;

/// How long the pre-takeoff wing flick lasts, in seconds.
pub const WARM_SECONDS: f32 = 0.45;

/// Upward velocity imparted by the jump, mm/s.
///
/// The flick is what makes the jump read as a jump; this is just how hard the
/// legs push. A real fly leaves the ground at a few hundred mm/s.
pub const JUMP_SPEED: f32 = 170.0;

pub struct Body {
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub gait: Gait,
    /// True wingbeat phase, radians. Advances at 200 Hz.
    pub wing_phase: f32,
    /// The phase the viewer should draw, advancing at `WING_VISUAL_HZ`.
    pub wing_phase_visual: f32,
    /// How hard the wings are working, 0–1.
    pub wing_power: f32,
    /// How folded the wings are, 0 = spread for flight, 1 = folded away. A
    /// walking fly keeps its wings flat over its back.
    pub wing_fold: f32,
    /// Position in the step cycle, 0–1. Frozen while airborne.
    pub leg_phase: f32,
    /// Position in the grooming rub, 0–1. Frozen unless grooming.
    pub groom_phase: f32,
    /// Seconds since the last gait change, for the report.
    pub gait_time: f32,
    /// Seconds since it last left the ground. Outlasts a single gait, so a
    /// fly that grooms and then walks and then grooms again still eventually
    /// gives up and flies off, instead of living on the food patch forever.
    pub ground_time: f32,
    /// How many times it has switched between flying and walking.
    pub gait_changes: u32,
    /// Times the fly has hit something.
    pub bumps: u32,
    pub distance_mm: f32,
    pub walked_mm: f32,
    /// Whether the body was in contact with a surface on the previous tick, so
    /// `bumps` can count encounters rather than milliseconds.
    touching: bool,
}

impl Body {
    pub fn new(pos: [f32; 3]) -> Self {
        Body {
            pos,
            vel: [CRUISE_SPEED, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            gait: Gait::Flying,
            wing_phase: 0.0,
            wing_phase_visual: 0.0,
            wing_power: 0.5,
            wing_fold: 0.0,
            leg_phase: 0.0,
            groom_phase: 0.0,
            gait_time: 0.0,
            ground_time: 0.0,
            gait_changes: 0,
            bumps: 0,
            distance_mm: 0.0,
            walked_mm: 0.0,
            touching: false,
        }
    }

    /// Unit vector along the body's forward axis (+x when yaw, pitch and roll
    /// are all zero).
    #[inline]
    pub fn forward(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        [cp * cy, cp * sy, sp]
    }

    /// Unit vector towards the animal's own left.
    ///
    /// The three axes are the columns of `Rz(yaw) · Ry(−pitch) · Rx(roll)`,
    /// written out rather than composed per call — this runs a few million
    /// times a second.
    #[inline]
    pub fn left(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        [-cy * sp * sr - sy * cr, cy * cr - sy * sp * sr, cp * sr]
    }

    #[inline]
    pub fn up(&self) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        [-cy * sp * cr + sy * sr, -sy * sp * cr - cy * sr, cp * cr]
    }

    /// The three body axes at once.
    #[inline]
    pub fn axes(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (self.forward(), self.left(), self.up())
    }

    pub fn speed(&self) -> f32 {
        let v = self.vel;
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    pub fn is_walking(&self) -> bool {
        self.gait == Gait::Walking
    }

    pub fn is_grooming(&self) -> bool {
        self.gait == Gait::Grooming
    }

    /// On the ground, whether standing still or stepping.
    pub fn is_grounded(&self) -> bool {
        self.gait.is_grounded()
    }

    pub fn is_warming(&self) -> bool {
        self.gait == Gait::Warming
    }

    fn to_ground(&mut self) {
        self.gait = Gait::Walking;
        self.gait_time = 0.0;
        self.ground_time = 0.0;
        self.gait_changes += 1;
        self.pitch = 0.0;
        self.roll = 0.0;
        self.vel[2] = 0.0;
        if self.pos[2] < STAND_HEIGHT {
            self.pos[2] = STAND_HEIGHT;
        }
    }

    fn to_grooming(&mut self) {
        self.gait = Gait::Grooming;
        self.gait_time = 0.0;
        self.gait_changes += 1;
        self.vel = [0.0, 0.0, 0.0];
    }

    fn to_warming(&mut self) {
        self.gait = Gait::Warming;
        self.gait_time = 0.0;
        self.gait_changes += 1;
        self.vel = [0.0, 0.0, 0.0];
    }

    fn to_air(&mut self) {
        self.gait = Gait::Flying;
        self.gait_time = 0.0;
        self.ground_time = 0.0;
        self.gait_changes += 1;
        // Up *and* along the body axis: a fly does not leave the ground
        // vertically, it leaves at an angle, which is why the wings were
        // beating before the legs pushed.
        let f = self.forward();
        self.vel[0] = f[0] * JUMP_SPEED * 0.35;
        self.vel[1] = f[1] * JUMP_SPEED * 0.35;
        self.vel[2] = JUMP_SPEED;
    }

    /// One millisecond.
    pub fn step(&mut self, cmd: Flight, world: &World, dt: f32, input: BodyInput) {
        let before = self.pos;
        self.gait_time += dt;

        if self.gait != Gait::Flying {
            self.ground_time += dt;
        }
        match self.gait {
            Gait::Flying => {
                self.step_flying(cmd, world, dt);
                // Feet down only once it is actually near the floor. The motor
                // aims the altitude loop at the ground while landing, so this
                // is a wait for the descent to finish, not a gate that can
                // stall: `step_flying` clamps altitude from below and the loop
                // drives it down at a real descent rate.
                if input.land && self.gait_time > 1.5 && self.pos[2] < STAND_HEIGHT + 2.0 {
                    self.to_ground();
                }
            }
            Gait::Walking => {
                self.step_walking(cmd, world, dt);
                if input.startle > STARTLE_TAKEOFF || self.ground_time > GROUND_BUDGET {
                    self.to_warming();
                } else if input.at_food && self.gait_time > FEED_BEFORE_GROOM {
                    self.to_grooming();
                }
            }
            Gait::Grooming => {
                self.step_grooming(dt);
                if input.startle > STARTLE_TAKEOFF {
                    // Startled mid-groom: it still gets its wings going first.
                    self.to_warming();
                } else if self.gait_time > GROOM_SECONDS {
                    // Back on its feet, but `ground_time` keeps running, so it
                    // will not stand on the food patch indefinitely.
                    self.gait = Gait::Walking;
                    self.gait_time = 0.0;
                    self.gait_changes += 1;
                }
            }
            Gait::Warming => {
                self.step_warming(dt);
                if self.gait_time > WARM_SECONDS {
                    self.to_air();
                }
            }
        }

        // --- walls, in both gaits ---
        let (gap, normal) = world.nearest_surface(self.pos);
        // Blend the standoff by how vertical the surface is: walls need half a
        // body length, floors and ceilings need only a body's height.
        //
        // Except while the fly is coiling for a jump, when it is *supposed* to
        // be below standing height. Holding it at `STAND_HEIGHT` there does not
        // protect anything — it just cancels the crouch, and the fly jumps from
        // a completely straight-legged stance, which is the one thing the run-up
        // exists to avoid.
        let floor_standoff = if self.gait == Gait::Warming {
            STAND_HEIGHT * 0.75
        } else {
            STAND_HEIGHT
        };
        let standoff = WALL_STANDOFF + (floor_standoff - WALL_STANDOFF) * normal[2].abs();
        let touching = gap < standoff;
        if touching {
            let push = standoff - gap;
            for k in 0..3 {
                self.pos[k] += normal[k] * push;
            }
            let into = self.vel[0] * normal[0] + self.vel[1] * normal[1] + self.vel[2] * normal[2];
            if into < 0.0 {
                for k in 0..3 {
                    self.vel[k] -= 2.0 * into * normal[k];
                }
                for v in &mut self.vel {
                    *v *= 0.55;
                }
                // Count *encounters*, not milliseconds of contact. A fly pinned
                // against a wall for a tenth of a second would otherwise score
                // a hundred, and the number stops meaning anything.
                if !self.touching {
                    self.bumps += 1;
                }
            }
        }
        self.touching = touching;

        // --- wings and legs ---
        self.wing_phase += std::f32::consts::TAU * WINGBEAT_HZ * dt;
        if self.wing_phase > std::f32::consts::TAU * 1000.0 {
            self.wing_phase -= std::f32::consts::TAU * 1000.0;
        }
        self.wing_phase_visual += std::f32::consts::TAU * WING_VISUAL_HZ * dt;

        // Wings come out for flight *and* for the run-up to it. Keeping them
        // folded until the instant of takeoff is precisely what made the jump
        // read as being thrown rather than as jumping.
        let wings_out = matches!(self.gait, Gait::Flying | Gait::Warming);
        let fold_target = if wings_out { 0.0 } else { 1.0 };
        // Faster during the flick: this is a run-up, not a glide.
        let fold_tau = if self.gait == Gait::Warming { 0.05 } else { 0.18 };
        self.wing_fold += (fold_target - self.wing_fold) * (dt / fold_tau).min(1.0);

        let power_target = match self.gait {
            Gait::Flying => 0.85,
            // Flat out. It is the whole point of the state.
            Gait::Warming => 1.0,
            _ => 0.05,
        };
        self.wing_power += (power_target - self.wing_power) * (dt / 0.15).min(1.0);

        match self.gait {
            Gait::Walking => self.leg_phase = (self.leg_phase + GAIT_HZ * dt).fract(),
            Gait::Grooming => self.groom_phase = (self.groom_phase + GROOM_HZ * dt).fract(),
            // Flying keeps both phases frozen; warming coils the legs, which
            // the viewer does from `gait` rather than from a phase.
            Gait::Flying | Gait::Warming => {}
        }

        let d = [
            self.pos[0] - before[0],
            self.pos[1] - before[1],
            self.pos[2] - before[2],
        ];
        let moved = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        self.distance_mm += moved;
        if self.is_walking() {
            self.walked_mm += moved;
        }
    }

    fn step_flying(&mut self, cmd: Flight, _world: &World, dt: f32) {
        // --- attitude ---
        self.yaw += cmd.yaw * dt;
        self.pitch = (self.pitch + cmd.pitch * dt).clamp(-0.7, 0.7);
        self.roll = (self.roll + cmd.roll * dt).clamp(-1.1, 1.1);
        // Bank into the turn, the way anything with wings does. Without this
        // the fly slides sideways like a hovercraft.
        let bank_target = (-cmd.yaw * 0.25).clamp(-0.8, 0.8);
        self.roll += (bank_target - self.roll) * (dt / 0.09).min(1.0);

        // --- linear ---
        let f = self.forward();
        self.vel[0] += f[0] * cmd.thrust * dt;
        self.vel[1] += f[1] * cmd.thrust * dt;
        self.vel[2] += f[2] * cmd.thrust * dt - GRAVITY * dt;

        let damp = (-dt / DRAG_TAU).exp();
        for v in &mut self.vel {
            *v *= damp;
        }

        for k in 0..3 {
            self.pos[k] += self.vel[k] * dt;
        }
        // Do not fly through the floor — but stop a little *below* standing
        // height rather than exactly at it.
        //
        // Clamping exactly at the standoff made the floor collision branch in
        // `step` unreachable: the body could never get closer to the floor
        // than the distance that would have triggered it, so a dive into the
        // ground was silently forgiven while the same dive into a wall was
        // counted and reflected. Clamping a bit lower lets the real collision
        // code see the contact, which is what it is for.
        if self.pos[2] < FLOOR_CLAMP {
            self.pos[2] = FLOOR_CLAMP;
        }
    }

    /// Standing still, wiping its face with its front legs.
    ///
    /// Nothing moves except the front legs. That is the entire point of the
    /// state: a fly that has stopped to clean itself is doing something that
    /// has no goal, which is precisely what makes it read as an animal rather
    /// than as a controller.
    fn step_grooming(&mut self, dt: f32) {
        self.pitch += (0.0 - self.pitch) * (dt / 0.12).min(1.0);
        self.roll += (0.0 - self.roll) * (dt / 0.12).min(1.0);
        self.vel = [0.0, 0.0, 0.0];
        self.pos[2] += (STAND_HEIGHT - self.pos[2]) * (dt / 0.10).min(1.0);
    }

    /// Wings out and beating hard, legs coiling — the run-up to the jump.
    ///
    /// Nothing translates: it is standing still, and the only things moving are
    /// the wings coming out of their folded position and the body sinking a
    /// little as the legs compress.
    fn step_warming(&mut self, dt: f32) {
        self.pitch += (0.0 - self.pitch) * (dt / 0.08).min(1.0);
        self.roll += (0.0 - self.roll) * (dt / 0.08).min(1.0);
        self.vel = [0.0, 0.0, 0.0];
        let t = (self.gait_time / WARM_SECONDS).min(1.0);
        // Eighteen percent down at the deepest, then the legs push.
        let crouch = STAND_HEIGHT * (1.0 - 0.18 * t);
        self.pos[2] += (crouch - self.pos[2]) * (dt / 0.05).min(1.0);
    }

    fn step_walking(&mut self, cmd: Flight, _world: &World, dt: f32) {
        // --- attitude: on the ground, level, and turned only about z ---
        self.yaw += cmd.yaw * dt;
        self.pitch += (0.0 - self.pitch) * (dt / 0.12).min(1.0);
        self.roll += (0.0 - self.roll) * (dt / 0.12).min(1.0);

        // --- linear: a walk drive along the body axis, and no lift at all ---
        // The caller sends the same "thrust" number in both gaits, so it is
        // first normalised against the cruise thrust that would balance drag
        // in the air, then re-scaled so that a full-scale command walks at
        // `WALK_SPEED`. Simply scaling by `WALK_SPEED / CRUISE_SPEED` — which
        // is what the first version did — leaves the steady state at a third of
        // the intended speed, because the two drag time constants differ.
        let f = self.forward();
        let cruise = CRUISE_SPEED / DRAG_TAU;
        let drive = (cmd.thrust / cruise).clamp(0.0, 2.0) * WALK_SPEED / WALK_TAU;
        self.vel[0] += f[0] * drive * dt;
        self.vel[1] += f[1] * drive * dt;
        let damp = (-dt / WALK_TAU).exp();
        self.vel[0] *= damp;
        self.vel[1] *= damp;
        self.vel[2] = 0.0;

        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
        self.pos[2] += (STAND_HEIGHT - self.pos[2]) * (dt / 0.10).min(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flying() -> (Body, World) {
        let w = World::new();
        let b = Body::new(w.start_position());
        (b, w)
    }

    /// A body already just above the floor, with the minimum flight time
    /// elapsed, so a landing command takes effect on the next tick.
    ///
    /// The *descent* is the motor's job and is tested there; these tests are
    /// about the gait state machine, and starting them at cruise altitude
    /// would only be testing the landing gate over and over.
    fn landing() -> (Body, World) {
        let w = World::new();
        let mut b = Body::new([-30.0, -20.0, STAND_HEIGHT + 1.0]);
        b.vel = [0.0, 0.0, 0.0];
        b.yaw = 0.0;
        b.gait_time = 2.0;
        (b, w)
    }

    #[test]
    fn the_body_axes_are_orthonormal() {
        let (mut b, _) = flying();
        b.yaw = 0.7;
        b.pitch = 0.3;
        b.roll = -0.4;
        let (f, l, u) = b.axes();
        for v in [f, l, u] {
            let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((n - 1.0).abs() < 1e-4, "axis not unit length: {v:?} ({n})");
        }
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        assert!(dot(f, l).abs() < 1e-4, "forward and left are not perpendicular");
        assert!(dot(f, u).abs() < 1e-4, "forward and up are not perpendicular");
        assert!(dot(l, u).abs() < 1e-4, "left and up are not perpendicular");
    }

    #[test]
    fn a_level_fly_faces_plus_x() {
        let (b, _) = flying();
        let f = b.forward();
        assert!((f[0] - 1.0).abs() < 1e-6, "{f:?}");
        assert!(f[1].abs() < 1e-6 && f[2].abs() < 1e-6, "{f:?}");
        let l = b.left();
        assert!((l[1] - 1.0).abs() < 1e-6, "{l:?}");
    }

    #[test]
    fn thrust_actually_moves_it_and_drag_bounds_the_speed() {
        let (mut b, w) = flying();
        b.vel = [0.0, 0.0, 0.0];
        let cmd = Flight { thrust: 2000.0, ..Default::default() };
        for _ in 0..300 {
            b.step(cmd, &w, 0.001, BodyInput::default());
        }
        let expected = 2000.0 * DRAG_TAU;
        assert!(
            (b.speed() - expected).abs() < expected * 0.25,
            "expected about {expected} mm/s, got {}",
            b.speed()
        );
    }

    #[test]
    fn it_cannot_leave_the_room() {
        let (mut b, w) = flying();
        b.yaw = 0.3;
        let cmd = Flight { thrust: 4000.0, yaw: 1.5, ..Default::default() };
        for _ in 0..20_000 {
            b.step(cmd, &w, 0.001, BodyInput::default());
            assert!(
                b.pos[0].abs() < crate::world::HALF_XY + 5.0
                    && b.pos[1].abs() < crate::world::HALF_XY + 5.0
                    && b.pos[2] < crate::world::HEIGHT + 5.0,
                "escaped the room at {:?}",
                b.pos
            );
        }
        assert!(b.bumps > 0, "a fly flying hard in a closed room must hit something");
    }

    #[test]
    fn the_visual_wing_phase_runs_slower_than_the_real_one() {
        let (mut b, w) = flying();
        for _ in 0..1000 {
            b.step(Flight::default(), &w, 0.001, BodyInput::default());
        }
        let real = b.wing_phase / std::f32::consts::TAU;
        let visual = b.wing_phase_visual / std::f32::consts::TAU;
        assert!((real - 200.0).abs() < 1.0, "real wingbeat {real}");
        assert!((visual - WING_VISUAL_HZ).abs() < 0.5, "visual wingbeat {visual}");
    }

    #[test]
    fn a_landing_command_puts_it_on_the_ground() {
        let (mut b, w) = landing();
        let cmd = Flight { thrust: CRUISE_SPEED / DRAG_TAU, ..Default::default() };
        for _ in 0..3000 {
            b.step(cmd, &w, 0.001, BodyInput { land: true, startle: 0.0, at_food: false });
        }
        assert!(b.is_walking(), "should have landed");
        assert!(
            (b.pos[2] - STAND_HEIGHT).abs() < 0.05,
            "should be standing at {STAND_HEIGHT} mm, got {}",
            b.pos[2]
        );
        assert!(b.wing_fold > 0.9, "wings should be folded away, got {}", b.wing_fold);
    }

    #[test]
    fn a_walking_fly_stays_on_the_ground_and_makes_progress() {
        // Landing takes at least 1.5 s, and at cruise that is 220 mm — more
        // than the room is wide. So this starts it low and slow in open space
        // rather than letting it fly into a corner and then walk on the spot
        // against a wall, which is what the first version of this test did.
        let w = World::new();
        let mut b = Body::new([-40.0, -20.0, STAND_HEIGHT]);
        b.vel = [0.0, 0.0, 0.0];
        b.yaw = 0.0; // facing +x, down the long axis of the room
        // Past the minimum flight time, so it puts its feet down at once
        // instead of flying across the room first. The behaviour layer is what
        // normally handles the approach; this test is about the walking.
        b.gait_time = 2.0;

        let cmd = Flight { thrust: CRUISE_SPEED / DRAG_TAU, ..Default::default() };
        let land = BodyInput { land: true, startle: 0.0, at_food: false };
        for _ in 0..200 {
            b.step(cmd, &w, 0.001, land);
        }
        assert!(b.is_walking(), "should have landed");
        let start = b.pos;

        for _ in 0..2000 {
            b.step(cmd, &w, 0.001, land);
        }
        assert!(b.pos[2] < STAND_HEIGHT + 0.05, "walking fly left the ground: z={}", b.pos[2]);
        assert!(b.is_walking(), "should still be walking");

        let moved = ((b.pos[0] - start[0]).powi(2) + (b.pos[1] - start[1]).powi(2)).sqrt();
        assert!(moved > 20.0, "a walking fly should cover ground, got {moved} mm");
        // And it should be doing roughly the speed it is supposed to, not
        // crawling in a circle.
        assert!(
            b.walked_mm > 30.0,
            "two seconds of walking should cover 50 mm, walked {}",
            b.walked_mm
        );
    }

    #[test]
    fn a_startle_makes_a_walking_fly_take_off() {
        let (mut b, w) = landing();
        let cmd = Flight { thrust: CRUISE_SPEED / DRAG_TAU, ..Default::default() };
        for _ in 0..3000 {
            b.step(cmd, &w, 0.001, BodyInput { land: true, startle: 0.0, at_food: false });
        }
        assert!(b.is_walking());
        let before = b.gait_changes;
        let jump = BodyInput { land: false, startle: STARTLE_TAKEOFF + 0.1, at_food: false };
        let quiet = BodyInput { land: false, startle: 0.0, at_food: false };

        // A startle does not jump; it starts the run-up. Every route from the
        // ground into the air goes through the wing flick.
        b.step(cmd, &w, 0.001, jump);
        assert!(b.is_warming(), "a startle should start the run-up, not the jump");

        let ticks = (WARM_SECONDS / 0.001) as usize + 20;
        for _ in 0..ticks {
            b.step(cmd, &w, 0.001, quiet);
        }
        assert!(b.gait == Gait::Flying, "should be airborne by now, is {:?}", b.gait);
        assert!(b.gait_changes >= before + 2, "walking → warming → flying is two changes");
        assert!(b.vel[2] > 0.0, "the jump should have upward velocity, got {}", b.vel[2]);
    }

    #[test]
    fn the_wings_are_out_and_working_before_the_feet_leave_the_ground() {
        // The visible content of the run-up. Without it the fly jumps with its
        // wings still folded, which is what it did before this state existed
        // and what made the takeoff look like a throw.
        let (mut b, w) = landing();
        let cmd = Flight { thrust: CRUISE_SPEED / DRAG_TAU, ..Default::default() };
        for _ in 0..3000 {
            b.step(cmd, &w, 0.001, BodyInput { land: true, startle: 0.0, at_food: false });
        }
        assert!(b.is_walking());
        assert!(b.wing_fold > 0.9, "a walking fly keeps its wings folded");

        let jump = BodyInput { land: false, startle: STARTLE_TAKEOFF + 0.1, at_food: false };
        b.step(cmd, &w, 0.001, jump);

        // The last few milliseconds of the flick, which is the moment that
        // matters: wings fully out and working *before* the feet leave. Check
        // it earlier and the power ramp is only partway up — which is correct
        // behaviour and a wrong assertion.
        let ticks = (WARM_SECONDS / 0.001) as usize - 5;
        for _ in 0..ticks {
            b.step(cmd, &w, 0.001, BodyInput { land: false, startle: 0.0, at_food: false });
        }
        assert!(b.is_warming(), "should still be on the ground mid-flick");
        assert!(b.wing_fold < 0.05, "wings should be out by now, fold = {}", b.wing_fold);
        assert!(b.wing_power > 0.9, "wings should be flat out, power = {}", b.wing_power);
        assert!(
            b.pos[2] < STAND_HEIGHT,
            "the legs should be coiling, height = {}",
            b.pos[2]
        );
    }

    #[test]
    fn the_tripod_gait_advances_only_while_walking() {
        let (mut b, w) = landing();
        b.leg_phase = 0.0;
        for _ in 0..100 {
            b.step(Flight::default(), &w, 0.001, BodyInput::default());
        }
        assert_eq!(b.leg_phase, 0.0, "legs should be still while flying");

        let cmd = Flight { thrust: CRUISE_SPEED / DRAG_TAU, ..Default::default() };
        for _ in 0..3000 {
            b.step(cmd, &w, 0.001, BodyInput { land: true, startle: 0.0, at_food: false });
        }
        let a = b.leg_phase;
        for _ in 0..500 {
            b.step(cmd, &w, 0.001, BodyInput { land: true, startle: 0.0, at_food: false });
        }
        assert!(b.leg_phase != a, "legs should be stepping while walking");
        assert!((0.0..1.0).contains(&b.leg_phase), "phase must stay in [0,1)");
    }
}
