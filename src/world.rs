//! The room the fly lives in.
//!
//! Millimetres, right-handed, `+x` forward, `+y` the animal's left, `+z` up.
//! Ground is the `z = 0` plane.
//!
//! The world is small on purpose. A real Drosophila is 2.5 mm long and cruises
//! at a few hundred millimetres per second, so a 200 mm room is about a second
//! of flight across — which is a fly's idea of a room, and also about as much
//! space as the camera can follow without the animal becoming a dot.

use crate::coord::FLY_LENGTH_MM;

/// Half-extents of the room in x and y, and its height. A 200 × 200 × 120 mm
/// box, or roughly eighty fly-lengths on a side.
pub const HALF_XY: f32 = 100.0;
pub const HEIGHT: f32 = 120.0;

#[derive(Clone, Copy, Debug)]
pub struct Pillar {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
}

/// Food patches, mm. Placed by hand to sit in the gaps between the pillar ring
/// and the walls, so the walk to one is never a walk into a pillar.
pub const FOOD: [[f32; 2]; 4] = [[-25.0, 35.0], [40.0, 25.0], [10.0, -55.0], [-48.0, -20.0]];

/// Radius of a patch, mm. About two fly-lengths across.
pub const FOOD_RADIUS: f32 = 5.0;

/// A pebble on the floor.
#[derive(Clone, Copy, Debug)]
pub struct Pebble {
    pub x: f32,
    pub y: f32,
    /// Radius, mm. Also its height above the floor to the centre.
    pub r: f32,
}

/// Floor debris. These are real obstacles — they are in `nearest_surface`, so
/// the fly walks around them.
///
/// They started out as pure decoration: drawn by the viewer from a hash of its
/// own loop index, absent from the simulation entirely. So the fly walked
/// straight through them while carefully avoiding the pillars, and the room
/// was full of objects that looked solid and were not. That is worse than
/// having no objects at all — it teaches the wrong thing about what the world
/// is. Anything the viewer draws as a solid thing has to exist here too.
pub const PEBBLE_COUNT: u32 = 12;

pub struct World {
    /// A bright spot the fly is drawn towards. Phototaxis is one of the few
    /// fly behaviours nobody argues about.
    pub light: [f32; 3],
    pub pillars: Vec<Pillar>,
    /// Food patches on the floor.
    ///
    /// These exist because a fly on a bare plane has nowhere to go. Once it had
    /// learned to land, the walking read as random drift — an animal wandering
    /// a featureless floor looks like a bug in the software, not like a bug.
    /// Giving it something to walk *to* is most of what makes the walking look
    /// purposeful, and it costs four circles.
    pub food: Vec<[f32; 2]>,
    /// Scattered floor debris. See `Pebble`.
    pub pebbles: Vec<Pebble>,
    /// Seconds elapsed, for anything that wants to change over time.
    pub t: f32,
}

impl World {
    pub fn new() -> Self {
        // Three pillars spread around the middle of the room, tall enough that
        // the fly has to go around them rather than over.
        let pillars: Vec<Pillar> = (0..3)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 3.0;
                Pillar {
                    x: 55.0 * a.cos(),
                    y: 55.0 * a.sin(),
                    radius: 7.0,
                }
            })
            .collect();

        // Stones first, so the generator can still see the pillars it has to
        // avoid — struct fields are evaluated in the order they are written,
        // and writing them into the literal would have moved `pillars` before
        // this ran.
        //
        // Seeded, so the room is the same room every run — and so the viewer
        // and the simulator are looking at the same stones.
        let mut rng = crate::rng::Rng::from_seed(0x_DEB_115);
        let mut pebbles: Vec<Pebble> = Vec::with_capacity(PEBBLE_COUNT as usize);
        // Bounded, in case the rejection sampling is ever asked for more
        // stones than the room has gaps for — a generator that can spin
        // forever is a generator that hangs on startup.
        let mut tries = 0;
        while pebbles.len() < PEBBLE_COUNT as usize && tries < PEBBLE_COUNT * 40 {
            tries += 1;
            let x = rng.range(-HALF_XY * 0.94, HALF_XY * 0.94);
            let y = rng.range(-HALF_XY * 0.94, HALF_XY * 0.94);
            // Big enough to see from the wide camera. At 0.25–0.65 they were
            // real obstacles that the fly steered around and that nobody could
            // tell were there, which is the same failure as before in the
            // opposite direction.
            let r = rng.range(0.5, 1.4);
            // Not on a food patch: the fly walks to those on purpose, and a
            // stone in the middle of one is a stone in the way of dinner.
            let on_food = FOOD
                .iter()
                .any(|f| ((x - f[0]).powi(2) + (y - f[1]).powi(2)).sqrt() < FOOD_RADIUS + r + 1.0);
            let in_pillar = pillars
                .iter()
                .any(|p| ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt() < p.radius + r + 0.5);
            // And not on top of another stone. Without this the placement is
            // pure rejection sampling with no separation, so two stones
            // eventually land in the same spot — they interpenetrate visually,
            // and the collision surface becomes the union of two spheres, which
            // is a shape nothing in the scene is actually drawing.
            let overlapping = pebbles.iter().any(|q| {
                ((x - q.x).powi(2) + (y - q.y).powi(2)).sqrt() < r + q.r + 2.0
            });
            if on_food || in_pillar || overlapping {
                continue;
            }
            pebbles.push(Pebble { x, y, r });
        }

        World {
            // Directly over the middle of the room. It was originally off to
            // one side at (60, 0, 95) — which put it 8 mm from a pillar, so the
            // only corridor to the lamp was blocked by one. The fly spent the
            // run pressed against that pillar, and the metrics said it was
            // reaching the light because the lamp was close and the pillar was
            // closer.
            light: [0.0, 0.0, 95.0],
            pillars,
            food: FOOD.iter().map(|f| [f[0], f[1]]).collect(),
            pebbles,
            t: 0.0,
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.t += dt;
    }

    /// Distance from a point to the nearest surface it could hit, and the
    /// outward direction of that surface. Used both for collision and for the
    /// looming signal, so the fly's alarm is about the same walls it can
    /// actually crash into.
    pub fn nearest_surface(&self, p: [f32; 3]) -> (f32, [f32; 3]) {
        let mut best = (f32::MAX, [0.0, 0.0, 1.0]);

        // Room walls. The fly is inside, so the surface it approaches is the
        // wall it is heading towards, and the outward direction is inward.
        let walls = [
            (HALF_XY - p[0], [-1.0, 0.0, 0.0]),
            (HALF_XY + p[0], [1.0, 0.0, 0.0]),
            (HALF_XY - p[1], [0.0, -1.0, 0.0]),
            (HALF_XY + p[1], [0.0, 1.0, 0.0]),
        ];
        for (d, dir) in walls {
            if d < best.0 {
                best = (d, dir);
            }
        }
        // Floor and ceiling.
        for (d, dir) in [(p[2], [0.0, 0.0, 1.0]), (HEIGHT - p[2], [0.0, 0.0, -1.0])] {
            if d < best.0 {
                best = (d, dir);
            }
        }

        // Pebbles, treated as spheres sitting on the floor — centred at z = r,
        // so they touch the ground and stand 2r proud of it.
        for k in &self.pebbles {
            let (dx, dy) = (p[0] - k.x, p[1] - k.y);
            let dz = p[2] - k.r;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-6);
            let d = dist - k.r;
            if d < best.0 {
                best = (d, [dx / dist, dy / dist, dz / dist]);
            }
        }

        // Pillars, treated as vertical cylinders.
        for k in &self.pillars {
            let (dx, dy) = (p[0] - k.x, p[1] - k.y);
            let d = (dx * dx + dy * dy).sqrt() - k.radius;
            if d < best.0 {
                let n = if d.abs() > 1e-6 {
                    [dx / (d + k.radius), dy / (d + k.radius), 0.0]
                } else {
                    [1.0, 0.0, 0.0]
                };
                best = (d, n);
            }
        }
        best
    }

    /// The nearest food patch: horizontal distance in mm, and the unit vector
    /// towards it. Returns distance infinity if there is none.
    pub fn nearest_food(&self, p: [f32; 3]) -> (f32, [f32; 3]) {
        let mut best = (f32::MAX, [1.0, 0.0, 0.0]);
        for f in &self.food {
            let (dx, dy) = (f[0] - p[0], f[1] - p[1]);
            let d = (dx * dx + dy * dy).sqrt();
            if d < best.0 {
                best = (d, [dx / d.max(1e-6), dy / d.max(1e-6), 0.0]);
            }
        }
        best
    }

    /// Unit vector from the fly towards the light.
    pub fn towards_light(&self, p: [f32; 3]) -> [f32; 3] {
        let d = [
            self.light[0] - p[0],
            self.light[1] - p[1],
            self.light[2] - p[2],
        ];
        let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
        [d[0] / n, d[1] / n, d[2] / n]
    }

    /// How visible the light is from a point, in [0, 1] — inverse-square falloff
    /// with a floor, so the room is never pitch black.
    pub fn light_intensity(&self, p: [f32; 3]) -> f32 {
        let v = [
            self.light[0] - p[0],
            self.light[1] - p[1],
            self.light[2] - p[2],
        ];
        let r2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
        // Inverse-square with a floor, so the far corner is dim but not dark.
        (4000.0 / (r2 + 500.0)).clamp(0.0, 1.0)
    }

    /// Where the fly starts: a corner, well clear of the pillar ring and as far
    /// from the light as the room allows, so the first thing it has to do is
    /// find it.
    pub fn start_position(&self) -> [f32; 3] {
        [-70.0, -70.0, 50.0]
    }

    /// The layout, for the viewer. Served rather than duplicated in
    /// JavaScript, so the picture and the physics cannot drift apart — a
    /// viewer with stale wall positions would draw the fly flying through
    /// pillars, which is exactly the kind of bug that gets blamed on the brain.
    pub fn to_json(&self) -> String {
        let pillars: Vec<String> = self
            .pillars
            .iter()
            .map(|p| format!("{{\"x\":{:.3},\"y\":{:.3},\"r\":{:.3}}}", p.x, p.y, p.radius))
            .collect();
        let food: Vec<String> = self
            .food
            .iter()
            .map(|f| format!("[{:.3},{:.3}]", f[0], f[1]))
            .collect();
        let pebbles: Vec<String> = self
            .pebbles
            .iter()
            .map(|k| format!("[{:.3},{:.3},{:.3}]", k.x, k.y, k.r))
            .collect();
        format!(
            "{{\"half_xy\":{:.3},\"height\":{:.3},\"light\":[{:.3},{:.3},{:.3}],\
             \"pillars\":[{}],\"food\":[{}],\"food_radius\":{:.3},\"pebbles\":[{}],\
             \"fly_length\":{:.3},\"start\":[{:.3},{:.3},{:.3}]}}",
            HALF_XY,
            HEIGHT,
            self.light[0],
            self.light[1],
            self.light[2],
            pillars.join(","),
            food.join(","),
            FOOD_RADIUS,
            pebbles.join(","),
            FLY_LENGTH_MM,
            self.start_position()[0],
            self.start_position()[1],
            self.start_position()[2],
        )
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanity: the room has to be big enough that the fly is not permanently
/// jammed against a wall.
#[allow(dead_code)]
pub const MIN_ROOM_IN_FLY_LENGTHS: f32 = HALF_XY * 2.0 / FLY_LENGTH_MM;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_centre_of_the_room_is_far_from_every_wall() {
        let w = World::new();
        let (d, _) = w.nearest_surface([0.0, 0.0, HEIGHT / 2.0]);
        assert!(d > 30.0, "centre should be roomy, got {d} mm");
    }

    #[test]
    fn a_point_on_the_floor_reports_the_floor() {
        let w = World::new();
        let (d, n) = w.nearest_surface([0.0, 0.0, 0.4]);
        assert!(d < 1.0, "{d}");
        assert!(n[2] > 0.5, "the way out of the floor is up, got {n:?}");
    }

    #[test]
    fn a_pillar_is_closer_than_the_wall_it_stands_near() {
        let w = World::new();
        let k = w.pillars[0];
        let just_outside = [k.x + k.radius + 2.0, k.y, 40.0];
        let (d, n) = w.nearest_surface(just_outside);
        assert!((d - 2.0).abs() < 0.1, "expected 2 mm from the pillar, got {d}");
        // The normal is the direction that gets you *out* of the surface, so
        // away from the pillar centre — the fly is outside the cylinder.
        assert!(n[0] > 0.0, "normal should point outward, into open space: {n:?}");
    }

    #[test]
    fn light_falls_off_with_distance_and_stays_bounded() {
        let w = World::new();
        let near = w.light_intensity([w.light[0] - 10.0, w.light[1], w.light[2]]);
        let far = w.light_intensity([-90.0, 90.0, 5.0]);
        assert!((0.0..=1.0).contains(&near) && (0.0..=1.0).contains(&far));
        assert!(near > far, "near {near} should beat far {far}");
        assert!(far > 0.0, "the room should never be pitch black, got {far}");
    }

    #[test]
    fn a_pebble_is_something_the_fly_can_actually_hit() {
        // This is the assertion whose absence let the floor debris be purely
        // visual for a day: the viewer drew forty-eight stones that the
        // simulation had never heard of, so the fly walked through all of them.
        // Anything drawn as a solid object has to be findable here.
        let w = World::new();
        assert_eq!(w.pebbles.len(), PEBBLE_COUNT as usize);
        for k in &w.pebbles {
            // From directly above, not from the side. Beside a stone lying on
            // the floor the nearest surface is *the floor*, which is a fact
            // about the room rather than about the stone — the first version of
            // this test asserted the wrong one and failed for the right reason.
            let above = [k.x, k.y, k.r + k.r + 1.5];
            let (d, n) = w.nearest_surface(above);
            assert!(
                (d - 1.5).abs() < 0.05,
                "a pebble at ({:.1}, {:.1}) r={:.2} should be 1.5 mm below, got {d}",
                k.x,
                k.y,
                k.r
            );
            // The convention everywhere in this file is that the normal points
            // *out of* the surface, into open space — the same as the floor
            // returning [0, 0, 1]. Hovering above a stone, that is up.
            assert!(n[2] > 0.5, "the normal should point up, away from the stone: {n:?}");
        }
    }

    #[test]
    fn the_pebbles_are_inside_the_room_and_not_on_the_food() {
        let w = World::new();
        for k in &w.pebbles {
            assert!(k.x.abs() < HALF_XY && k.y.abs() < HALF_XY, "pebble outside the room");
            assert!(k.r > 0.2 && k.r < 2.0, "pebble r={} is not a pebble", k.r);
            // A stone sitting on a food patch would be a stone the fly is
            // trying to walk to.
            for f in &w.food {
                let d = ((k.x - f[0]).powi(2) + (k.y - f[1]).powi(2)).sqrt();
                assert!(d > FOOD_RADIUS + k.r, "pebble sits on a food patch");
            }
        }
    }

    #[test]
    fn the_fly_starts_inside_the_room_and_off_the_pillars() {
        let w = World::new();
        let p = w.start_position();
        let (d, _) = w.nearest_surface(p);
        assert!(d > 20.0, "start position is only {d} mm from something");
    }
}
