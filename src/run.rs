//! The closed loop: world → senses → brain → descending neurons → body → world.
//!
//! Headless, so that the whole thing can be checked without a browser in the
//! way. If the fly cannot fly with no graphics attached, it will not fly with
//! them either; the graphics only make the failure prettier.

use anyhow::{Context, Result};
use std::path::Path;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::body::Body;
use crate::circuit::Circuit;
use crate::etl::GROUP_NAMES;
use crate::http::{self, Shared};
use crate::lif::{Params, Sim};
use crate::motor::Motor;
use crate::senses::Senses;
use crate::wiring::Wiring;
use crate::world::World;

/// How often the shared snapshot is refreshed, in ticks. About 60 Hz, which is
/// as often as any viewer can use it and often enough that a spike never
/// arrives in the wrong frame.
const PUBLISH_EVERY: u64 = 16;

/// How often the per-group firing rates are recomputed, in frames. Summing 21
/// groups walks the whole 139k-neuron population, which is not worth doing at
/// 60 Hz for a readout nobody can read that fast.
const RATES_EVERY_FRAMES: u32 = 4;

pub struct RunOpts {
    pub seconds: f64,
    /// Seconds between status lines. Zero prints only the summary.
    pub report_every: f64,
    /// Seed. The circuit is stochastic, so this is the only handle on
    /// reproducibility the whole system has.
    pub seed: u64,
    /// Turn the reflex off to see what the brain alone does. Worth looking at
    /// once: it is not a fly.
    pub reflex: f32,
    /// Multiplier on how far the descending neurons are allowed to move the
    /// controls. Zero gives a fly with a brain that is running but not
    /// connected, which is the baseline every tuning pass should start from.
    pub brain: f32,
    /// Address to serve the viewer on, or None to run headless.
    pub serve: Option<String>,
    /// When serving, hold the simulation to the wall clock instead of letting
    /// it run at whatever speed it can manage. One core has the headroom, and
    /// a fly that moves at 1.2× looks subtly wrong in a way nobody can name.
    pub realtime: bool,
    pub open: bool,
}

impl Default for RunOpts {
    fn default() -> Self {
        RunOpts {
            seconds: 20.0,
            report_every: 2.0,
            seed: 0x_F1_1E,
            reflex: 1.0,
            brain: 1.0,
            serve: None,
            realtime: true,
            open: false,
        }
    }
}

/// Copy the current frame into the shared snapshot for the HTTP server.
///
/// `pending` is every spike since the last publish, not just the most recent
/// tick's. Publishing happens once per 16 ms of simulated time, so passing
/// `sim.fired` directly would throw away fifteen sixteenths of the brain's
/// activity — which is exactly what the first version did, and it made a
/// 70,000-spike-per-second brain look like a 1,200-spike-per-second one.
#[allow(clippy::too_many_arguments)]
fn publish(
    sh: &Arc<Mutex<Shared>>,
    tick: u64,
    sim: &Sim,
    body: &Body,
    senses: &Senses,
    motor: &Motor,
    world: &World,
    group_idx: Option<&[Vec<u32>]>,
    pending: &[u32],
) {
    let mut s = match sh.lock() {
        Ok(s) => s,
        Err(p) => p.into_inner(),
    };
    s.tick = tick as u32;
    s.fly_pos = body.pos;
    s.yaw = body.yaw;
    s.pitch = body.pitch;
    s.roll = body.roll;
    s.wing_phase = body.wing_phase_visual;
    s.wing_power = body.wing_power;
    s.speed = body.speed();
    s.bumps = body.bumps;
    s.eye_left = senses.state.eye_left;
    s.eye_right = senses.state.eye_right;
    s.motion = senses.state.motion;
    s.loom = senses.state.loom;
    s.dn_left = motor.state.dn_left_hz;
    s.dn_right = motor.state.dn_right_hz;
    s.yaw_cmd = motor.state.yaw_cmd;
    s.thrust_cmd = motor.state.thrust;
    s.gait = body.gait.code();
    s.leg_phase = body.leg_phase;
    s.groom_phase = body.groom_phase;
    s.wing_fold = body.wing_fold;
    s.walked_mm = body.walked_mm;
    s.gait_changes = body.gait_changes;
    s.world_t = world.t;
    if let Some(group_idx) = group_idx {
        if s.group_hz.len() != group_idx.len() {
            s.group_hz = vec![0.0; group_idx.len()];
        }
        for (k, idx) in group_idx.iter().enumerate() {
            s.group_hz[k] = sim.group_hz(idx);
        }
    }
    // `push_spikes` stamps entries with `s.tick`, which was set above, so the
    // order of these two lines matters.
    s.push_spikes(pending);
}

pub fn run(data_dir: &Path, opts: RunOpts) -> Result<()> {
    let c = Circuit::load(&data_dir.join("brain.bin")).context("loading brain.bin")?;
    c.validate()?;
    let w = Wiring::new(&c);

    println!(
        "brain      {} neurons, {} edges",
        c.n(),
        c.n_edges()
    );
    println!(
        "wiring     {} photoreceptors ({} left / {} right)",
        w.counts.photoreceptor,
        w.photoreceptor_left.len(),
        w.photoreceptor_right.len()
    );
    println!("           {} motion detectors (T4 + T5)", w.counts.motion);
    println!("           {} looming detectors (LPLC2 + LC4)", w.counts.looming);
    println!(
        "           {} output neurons ({} descending: {} left / {} right, plus motor)",
        w.counts.descending,
        w.descending_left.len() + w.descending_right.len(),
        w.descending_left.len(),
        w.descending_right.len()
    );
    println!(
        "           driven: {} of {} neurons ({:.1}% of the brain is addressed by the world;",
        w.counts.total(),
        c.n(),
        100.0 * w.counts.total() as f32 / c.n() as f32
    );
    println!("           the remaining {:.1}% is left to be whatever the connectome makes of it)",
        100.0 - 100.0 * w.counts.total() as f32 / c.n() as f32);
    println!();

    let mut sim = Sim::new(&c, Params::default(), opts.seed);
    let mut world = World::new();
    let mut body = Body::new(world.start_position());
    let mut senses = Senses::default();
    let mut motor = Motor::default();
    motor.reflex = opts.reflex;
    motor.brain_yaw *= opts.brain;
    motor.brain_thrust *= opts.brain;

    // One tick is one millisecond — the neural time step and the integration
    // step are deliberately the same thing, so there is no interpolation
    // anywhere in the loop.
    let dt: f32 = 0.001;
    // Zero or negative means "until interrupted", which is how the viewer is
    // meant to be run.
    let ticks = if opts.seconds <= 0.0 {
        u64::MAX
    } else {
        (opts.seconds / dt as f64) as u64
    };
    let report_ticks = (opts.report_every / dt as f64) as u64;

    // --- the viewer, if anyone is watching ---
    let group_idx: Vec<Vec<u32>> =
        (0..GROUP_NAMES.len()).map(|g| c.by_group(g as u8)).collect();
    let shared = opts.serve.as_ref().map(|_| {
        Arc::new(Mutex::new(Shared {
            world_json: world.to_json(),
            ..Default::default()
        }))
    });
    if let (Some(addr), Some(sh)) = (opts.serve.clone(), shared.clone()) {
        let listener = TcpListener::bind(&addr).with_context(|| format!("binding {addr}; choose another --serve port"))?;
        let bound = listener.local_addr()?;
        let data = data_dir.to_path_buf();
        let n_groups = GROUP_NAMES.len();

        std::thread::Builder::new()
            .name("http".into())
            .spawn(move || {
                if let Err(e) = http::serve(listener, sh, data, n_groups) {
                    eprintln!("http server stopped: {e:#}");
                }
            })
            .context("spawning the http thread")?;
        let url = format!("http://{bound}/");
        println!("viewer     {url}");
        if opts.open { crate::platform::open_browser(&url); }
        println!();
    }

    // How well the fly is actually homing, as opposed to how it looks while
    // moving. A fly that wanders convincingly and never arrives is a failure
    // that is invisible in the position columns, so it gets its own.
    let mut closest_mm = f32::MAX;
    let mut ticks_near = 0u64;
    let mut ticks_walking = 0u64;

    println!(
        "{:>7} {:>7} {:>7} {:>8} {:>7} {:>6} {:>6} {:>7} {:>7} {:>7}",
        "t(s)", "x", "y", "z", "speed", "gait", "bumps", "light", "DN-L", "DN-R"
    );
    println!("{}", "-".repeat(82));

    let mut last_flight = crate::body::Flight::default();
    let wall_start = Instant::now();
    let mut frames: u32 = 0;
    let mut pending: Vec<u32> = Vec::with_capacity(8192);
    for tick in 1..=ticks {
        world.step(dt);
        senses.apply(&mut sim, &w, &body, &world);
        sim.step(&c);
        if shared.is_some() { pending.extend_from_slice(&sim.fired); }
        let (flight, input) = motor.read(&sim, &w, &body, &world, &senses, dt);
        last_flight = flight;
        body.step(flight, &world, dt, input);

        let d = motor.state.dist_to_light;
        if d < closest_mm {
            closest_mm = d;
        }
        if d < 30.0 {
            ticks_near += 1;
        }
        if body.is_walking() {
            ticks_walking += 1;
        }

        if let Some(sh) = &shared {
            if tick % PUBLISH_EVERY == 0 {
                frames += 1;
                // Rates are the expensive part (they walk all 139k neurons), so
                // they refresh on their own slower schedule while the pose and
                // the spike list go out every frame.
                let want_rates = frames % RATES_EVERY_FRAMES == 1;
                publish(
                    sh,
                    tick,
                    &sim,
                    &body,
                    &senses,
                    &motor,
                    &world,
                    want_rates.then_some(&group_idx[..]),
                    &pending,
                );
                pending.clear();

                // Hold to the wall clock. The simulator can outrun it by about
                // 1.2×, and letting it do so would make the fly fly fast for no
                // reason and burn the CPU the browser wants.
                if opts.realtime {
                    let behind = tick as f64 * dt as f64 - wall_start.elapsed().as_secs_f64();
                    if behind > 0.0 {
                        std::thread::sleep(Duration::from_secs_f64(behind.min(0.05)));
                    }
                }
            }
        }

        if report_ticks > 0 && tick % report_ticks == 0 {
            let m = &motor.state;
            println!(
                "{:>7.1} {:>7.1} {:>7.1} {:>8.1} {:>7.1} {:>6} {:>6} {:>7.1} {:>7.2} {:>7.2}",
                tick as f32 * dt,
                body.pos[0],
                body.pos[1],
                body.pos[2],
                body.speed(),
                match body.gait {
                    crate::body::Gait::Flying => "fly",
                    crate::body::Gait::Walking => "walk",
                    crate::body::Gait::Grooming => "groom",
                    crate::body::Gait::Warming => "warm",
                },
                body.bumps,
                m.dist_to_light,
                m.dn_left_hz,
                m.dn_right_hz,
            );
        }
    }

    let v = sim.vitals();
    println!();
    println!("== after {:.0} s of flight ==", opts.seconds);
    println!(
        "position      ({:.1}, {:.1}, {:.1}) mm    speed {:.0} mm/s",
        body.pos[0],
        body.pos[1],
        body.pos[2],
        body.speed()
    );
    println!(
        "distance      {:.0} mm travelled ({:.0} mm of it on foot), {} gait changes, {} wall contacts",
        body.distance_mm, body.walked_mm, body.gait_changes, body.bumps
    );
    println!(
        "light         closest approach {:.1} mm; {:.1}% of the time within 30 mm",
        closest_mm,
        100.0 * ticks_near as f64 / ticks as f64
    );
    println!(
        "gait          {:.1}% of the run on foot, {} takeoffs",
        100.0 * ticks_walking as f64 / ticks as f64,
        body.gait_changes
    );
    println!(
        "brain         {:.3} Hz mean, {:.1}% of neurons above 0.5 Hz, {} spikes total",
        v.mean_hz,
        100.0 * v.active as f32 / c.n() as f32,
        v.total_spikes
    );
    println!(
        "output        descending left {:.3} Hz, right {:.3} Hz → yaw {:+.3} rad/s",
        motor.state.dn_left_hz, motor.state.dn_right_hz, last_flight.yaw
    );
    println!(
        "                 thrust {:.0} mm/s², pitch {:+.3} rad/s",
        last_flight.thrust, last_flight.pitch
    );

    if !body.pos.iter().all(|v| v.is_finite()) {
        anyhow::bail!("the fly's position went non-finite — something diverged");
    }
    if v.mean_hz <= 0.0 {
        anyhow::bail!("the brain went completely silent");
    }
    if v.mean_hz > 200.0 {
        anyhow::bail!("the brain saturated at {:.1} Hz — the operating point is wrong", v.mean_hz);
    }
    Ok(())
}
