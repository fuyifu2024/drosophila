//! drosophila — a 3D world containing a fruit fly whose brain is the real
//! FlyWire connectome.
//!
//! Three subcommands, in the order you have to run them:
//!
//!   etl    FlyWire CSV dumps → data/brain.bin, points.bin, meshes.bin
//!   bench  how much of a Raspberry Pi does 139k neurons at 1 kHz actually cost?
//!   run    the simulation itself (and, with --serve, the HTTP bridge)

mod bench;
mod body;
mod circuit;
mod coord;
mod etl;
mod http;
mod lif;
mod motor;
mod platform;
mod rng;
mod run;
mod senses;
mod wiring;
mod world;

use anyhow::Result;
use std::path::PathBuf;

fn usage() {
    println!(
        "drosophila — a fruit fly with the FlyWire whole-brain connectome

usage: drosophila <command> [--data <dir>]

commands:
  etl     build brain.bin / points.bin / meshes.bin from the CSV dumps
  stats   read brain.bin back, validate it, and print what is in it
  bench   measure simulation cost per simulated second
  run     run the simulation
            --seconds N     how long to fly (default 5, use 0 for forever)
            --serve PORT    also serve the 3D viewer on 127.0.0.1:PORT
            --brain F       how far the connectome may move the controls (1.0)
            --reflex F      how much hard-wired flying to mix in (1.0)
            --open          open the viewer in your default browser
            --free-run      do not hold the simulation to the wall clock

Data is resolved beside the executable, then ./data, then the source tree.
Use --data DIR to select an explicit directory. Stop a simulation with Ctrl+C."
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--data <dir>` anywhere on the line, everything else is the command.
    let mut data_dir = platform::data_dir();
    let mut seconds = 5.0f64;
    let mut brain = 1.0f32;
    let mut reflex = 1.0f32;
    let mut serve: Option<String> = None;
    let mut realtime = true;
    let mut open = false;
    let mut positional: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data" => {
                i += 1;
                match args.get(i) {
                    Some(p) => data_dir = PathBuf::from(p),
                    None => {
                        eprintln!("--data needs a directory");
                        std::process::exit(2);
                    }
                }
            }
            "--seconds" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(v) => seconds = v,
                    None => {
                        eprintln!("--seconds needs a number");
                        std::process::exit(2);
                    }
                }
            }
            "--brain" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(v) => brain = v,
                    None => {
                        eprintln!("--brain needs a number");
                        std::process::exit(2);
                    }
                }
            }
            "--reflex" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(v) => reflex = v,
                    None => {
                        eprintln!("--reflex needs a number");
                        std::process::exit(2);
                    }
                }
            }
            "--serve" => {
                i += 1;
                match args.get(i) {
                    Some(a) => serve = Some(if a.contains(':') {
                        a.clone()
                    } else {
                        format!("127.0.0.1:{a}")
                    }),
                    None => {
                        eprintln!("--serve needs a port or address");
                        std::process::exit(2);
                    }
                }
            }
            "--free-run" => realtime = false,
            "--open" => open = true,
            "--help" | "-h" => { usage(); return Ok(()); }
            "--version" | "-V" => { println!("drosophila {}", env!("CARGO_PKG_VERSION")); return Ok(()); }
            other if other.starts_with('-') => anyhow::bail!("unknown option {other:?}; see --help"),
            other => positional.push(other),
        }
        i += 1;
    }

    anyhow::ensure!(seconds.is_finite() && seconds >= 0.0, "--seconds must be finite and nonnegative");
    anyhow::ensure!(brain.is_finite() && brain >= 0.0, "--brain must be finite and nonnegative");
    anyhow::ensure!(reflex.is_finite() && reflex >= 0.0, "--reflex must be finite and nonnegative");
    anyhow::ensure!(positional.len() <= 1, "expected one command; see --help");
    anyhow::ensure!(!open || serve.is_some(), "--open requires --serve PORT");
    if let Some(addr) = &serve {
        let socket: std::net::SocketAddr = addr.parse().map_err(|_| anyhow::anyhow!("--serve needs a port or loopback IP:port"))?;
        anyhow::ensure!(socket.ip().is_loopback(), "the viewer binds to loopback only");
    }
    match positional.first().copied() {
        Some("etl") => etl::run(&data_dir),
        Some("stats") => {
            let path = data_dir.join("brain.bin");
            let c = circuit::Circuit::load(&path)?;
            c.validate()?;
            print!("{}", c.report());
            let labels = std::fs::read_to_string(data_dir.join("labels.json"))?;
            println!("\nlabels.json:\n{}", labels.trim());
            Ok(())
        }
        Some("bench") => bench::run(&data_dir, seconds),
        Some("run") => run::run(
            &data_dir,
            run::RunOpts {
                seconds,
                brain,
                reflex,
                serve,
                realtime,
                open,
                ..Default::default()
            },
        ),
        Some(other) => anyhow::bail!("unknown command {other:?}; see --help"),
        None => { usage(); Ok(()) },
    }
}
