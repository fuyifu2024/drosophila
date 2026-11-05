//! The bridge to the viewer: a hand-written HTTP/1.1 server on loopback.
//!
//! Plain HTTP rather than WebSocket, for one reason that has nothing to do with
//! elegance: `curl http://127.0.0.1:8770/state | xxd | head` is a complete
//! debugging session. A framing protocol would have needed client code before
//! it could be inspected at all.
//!
//! The simulator runs on one thread and the server on another. They share one
//! mutex-guarded snapshot, written once per rendered frame and read once per
//! request. Nothing else crosses between them, which means a slow browser
//! cannot stall the brain and a slow brain cannot drop a connection.
//!
//! # `/state` wire format
//!
//! Little-endian throughout. Fixed prefix, then a variable spike list:
//!
//! ```text
//!   0   u32   magic 'DSTA'
//!   4   u32   tick
//!   8   f32×3 fly position, mm
//!  20   f32   yaw, pitch, roll
//!  32   f32   wing phase (visual), wing power, speed
//!  44   u32   wall contacts
//!  48   f32   eye left, eye right
//!  56   f32×4 image motion: fore, aft, down, up
//!  72   f32   looming
//!  76   f32   descending rate left, right
//!  84   f32   yaw command, thrust command
//!  92   u32   gait (0 flying, 1 walking, 2 grooming)
//!  96   f32   leg phase, groom phase, wing fold, walked distance
//! 112   u32   gait changes
//! 116   f32   world time
//! 120   u32   group count G
//! 124   f32×G per-group firing rate, Hz
//!  ...  u32   spike count N
//!  ...  u32×N neuron indices that fired since `since`
//! ```
//!
//! `?since=<tick>` returns only the spikes that fired after that tick. The
//! viewer passes back the tick it last saw, so a dropped or delayed response
//! costs it nothing but a slightly longer list — there is no session state on
//! the server side to get out of sync.

use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long spikes are kept, in ticks. At a 60 Hz viewer poll this is four
/// frames of slack; at 30 Hz it is two. Beyond that the viewer would rather
/// lose a few spikes than have the buffer grow without bound.
const SPIKE_HISTORY_TICKS: u32 = 250;

/// Ceiling on how many spikes one response may carry. A stimulating burst can
/// briefly push the brain past 500k spikes/s, and a 2 MB response every frame
/// would be worse than dropping some.
const MAX_SPIKES_PER_RESPONSE: usize = 40_000;

pub const STATE_MAGIC: u32 = 0x4154_5344; // "DSTA"

/// What the simulator publishes each frame and the server hands out.
#[derive(Default)]
pub struct Shared {
    pub tick: u32,
    pub fly_pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub wing_phase: f32,
    pub wing_power: f32,
    pub speed: f32,
    pub bumps: u32,
    pub eye_left: f32,
    pub eye_right: f32,
    pub motion: [f32; 4],
    pub loom: f32,
    pub dn_left: f32,
    pub dn_right: f32,
    pub yaw_cmd: f32,
    pub thrust_cmd: f32,
    /// 0 = flying, 1 = walking, 2 = grooming. Matches `body::Gait::code`.
    pub gait: u32,
    /// Step-cycle position, 0–1. Frozen while airborne.
    pub leg_phase: f32,
    /// Rub-cycle position, 0–1. Frozen unless grooming.
    pub groom_phase: f32,
    /// Wing fold, 0 = spread for flight, 1 = folded away.
    pub wing_fold: f32,
    pub walked_mm: f32,
    pub gait_changes: u32,
    pub world_t: f32,
    pub group_hz: Vec<f32>,
    pub spikes: VecDeque<(u32, u32)>,
    /// The room's layout, as JSON, filled in once at startup.
    pub world_json: String,
}

impl Shared {
    pub fn push_spikes(&mut self, fired: &[u32]) {
        for &n in fired {
            self.spikes.push_back((self.tick, n));
        }
        let cutoff = self.tick.saturating_sub(SPIKE_HISTORY_TICKS);
        while let Some(&(t, _)) = self.spikes.front() {
            if t < cutoff {
                self.spikes.pop_front();
            } else {
                break;
            }
        }
    }

    /// Serialise for `/state?since=`.
    pub fn encode(&self, since: u32, group_count: usize) -> Vec<u8> {
        let fresh: Vec<u32> = self
            .spikes
            .iter()
            .filter(|(t, _)| *t > since)
            .take(MAX_SPIKES_PER_RESPONSE)
            .map(|(_, n)| *n)
            .collect();

        let g = group_count.min(self.group_hz.len());
        let mut out = Vec::with_capacity(100 + g * 4 + 4 + fresh.len() * 4);

        put_u(&mut out, STATE_MAGIC);
        put_u(&mut out, self.tick);
        for k in 0..3 {
            put_f(&mut out, self.fly_pos[k]);
        }
        put_f(&mut out, self.yaw);
        put_f(&mut out, self.pitch);
        put_f(&mut out, self.roll);
        put_f(&mut out, self.wing_phase);
        put_f(&mut out, self.wing_power);
        put_f(&mut out, self.speed);
        put_u(&mut out, self.bumps);
        put_f(&mut out, self.eye_left);
        put_f(&mut out, self.eye_right);
        for k in 0..4 {
            put_f(&mut out, self.motion[k]);
        }
        put_f(&mut out, self.loom);
        put_f(&mut out, self.dn_left);
        put_f(&mut out, self.dn_right);
        put_f(&mut out, self.yaw_cmd);
        put_f(&mut out, self.thrust_cmd);
        put_u(&mut out, self.gait);
        put_f(&mut out, self.leg_phase);
        put_f(&mut out, self.groom_phase);
        put_f(&mut out, self.wing_fold);
        put_f(&mut out, self.walked_mm);
        put_u(&mut out, self.gait_changes);
        put_f(&mut out, self.world_t);
        put_u(&mut out, g as u32);
        for k in 0..g {
            put_f(&mut out, self.group_hz[k]);
        }
        put_u(&mut out, fresh.len() as u32);
        for n in &fresh {
            put_u(&mut out, *n);
        }
        out
    }
}

#[inline]
fn put_f(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn put_u(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

// Assets travel with the executable. No build-machine source path is needed.
fn viewer_asset(path: &str) -> Option<&'static [u8]> {
    match path {
        "/app.js" => Some(include_bytes!("../viewer/app.js")),
        "/model.js" => Some(include_bytes!("../viewer/model.js")),
        "/world_view.js" => Some(include_bytes!("../viewer/world_view.js")),
        "/protocol.js" => Some(include_bytes!("../viewer/protocol.js")),
        "/brain_view.js" => Some(include_bytes!("../viewer/brain_view.js")),
        "/vendor/three.module.js" => Some(include_bytes!("../viewer/vendor/three.module.js")),
        "/vendor/three.core.js" => Some(include_bytes!("../viewer/vendor/three.core.js")),
        _ => None,
    }
}

pub fn serve(
    listener: TcpListener,
    shared: Arc<Mutex<Shared>>,
    data_dir: PathBuf,
    group_count: usize,
) -> Result<()> {

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let shared = shared.clone();
        let data_dir = data_dir.clone();
        // One thread per connection, and not for throughput — for liveness.
        // Chromium opens several connections at once and some of them are
        // speculative: it connects and then sends nothing. A server that
        // handles connections one at a time blocks on the first idle socket
        // and the real requests sit in the backlog behind it. That is exactly
        // what happened: the page hung on "connecting to the simulator"
        // indefinitely, with no error anywhere, because every fetch it was
        // waiting on was queued behind a socket nobody was going to write to.
        std::thread::spawn(move || {
            let mut stream = stream;
            // A browser that opens a socket and then goes away must not leave
            // the thread alive forever either.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
            if let Err(e) = handle(&mut stream, &shared, &data_dir, group_count) {
                eprintln!("http: {e:#}");
            }
        });
    }
    Ok(())
}

fn handle(
    stream: &mut TcpStream,
    shared: &Arc<Mutex<Shared>>,
    data_dir: &Path,
    group_count: usize,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    // Drain the headers; we do not use any of them, but leaving them in the
    // socket confuses nothing here and everything after us.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or("GET");
    let target = parts.next().unwrap_or("/");
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (target, ""),
    };

    match path {
        "/" | "/index.html" => write_response(stream, 200, "text/html; charset=utf-8", include_bytes!("../viewer/index.html"))?,
        // Answered so the browser's automatic request does not show up as an
        // error in the console, where it would sit next to real ones.
        "/favicon.ico" => write_response(stream, 204, "image/x-icon", b"")?,
        "/state" => {
            let since: u32 = query
                .split('&')
                .find_map(|kv| kv.strip_prefix("since="))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let body = match shared.lock() {
                Ok(s) => s.encode(since, group_count),
                Err(p) => p.into_inner().encode(since, group_count),
            };
            write_response(stream, 200, "application/octet-stream", &body)?;
        }
        "/world.json" => {
            let body = match shared.lock() {
                Ok(s) => s.world_json.clone(),
                Err(p) => p.into_inner().world_json.clone(),
            };
            write_response(stream, 200, "application/json", body.as_bytes())?;
        }
        "/points.bin" => serve_file(stream, &data_dir.join("points.bin"), "application/octet-stream")?,
        "/meshes.bin" => serve_file(stream, &data_dir.join("meshes.bin"), "application/octet-stream")?,
        "/labels.json" => serve_file(stream, &data_dir.join("labels.json"), "application/json")?,
        other => match viewer_asset(other) {
            Some(body) => write_response(stream, 200, "text/javascript", body)?,
            None => write_response(stream, 404, "text/plain", b"not found\n")?,
        },
    }
    Ok(())
}

fn serve_file(stream: &mut TcpStream, path: &Path, content_type: &str) -> Result<()> {
    match std::fs::read(path) {
        Ok(body) => write_response(stream, 200, content_type, &body),
        Err(_) => write_response(
            stream,
            404,
            "text/plain",
            b"not found\n",
        ),
    }
}

fn write_response(stream: &mut TcpStream, status: u16, ct: &str, body: &[u8]) -> Result<()> {
    let reason = match status { 200 => "OK", 204 => "No Content", _ => "Not Found" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    Ok(())
}

/// A convenience for the viewer's development: fetch one state frame and print
/// a summary. Not used by the running server.
#[allow(dead_code)]
pub fn fetch_state(addr: &str, since: u32) -> Result<Vec<u8>> {
    let mut s = TcpStream::connect(addr)?;
    write!(
        s,
        "GET /state?since={since} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("no header terminator")?;
    Ok(buf[split + 4..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_frame_round_trips_through_the_encoder() {
        let mut s = Shared::default();
        s.tick = 1234;
        s.fly_pos = [1.5, -2.5, 30.0];
        s.yaw = 0.25;
        s.loom = 3.5;
        s.group_hz = vec![1.0, 2.0, 3.0, 4.0];
        s.gait = 1;
        s.leg_phase = 0.625;
        s.wing_fold = 0.5;
        s.spikes.push_back((1200, 7)); // too old: before `since`
        s.spikes.push_back((1230, 11));
        s.spikes.push_back((1234, 13));

        let b = s.encode(1220, 4);
        let rd_u32 = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let rd_f32 = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());

        assert_eq!(rd_u32(0), STATE_MAGIC);
        assert_eq!(rd_u32(4), 1234);
        assert_eq!(rd_f32(8), 1.5);
        assert_eq!(rd_f32(12), -2.5);
        assert_eq!(rd_f32(16), 30.0);
        assert_eq!(rd_f32(20), 0.25);
        assert_eq!(rd_f32(72), 3.5);
        assert_eq!(rd_u32(92), 1, "gait");
        assert_eq!(rd_f32(96), 0.625, "leg phase");
        assert_eq!(rd_f32(104), 0.5, "wing fold");
        assert_eq!(rd_u32(120), 4, "group count");
        assert_eq!(rd_f32(124), 1.0);
        assert_eq!(rd_f32(136), 4.0);
        let n_at = 124 + 4 * 4;
        assert_eq!(rd_u32(n_at), 2, "only the two spikes after tick 1220");
        assert_eq!(rd_u32(n_at + 4), 11);
        assert_eq!(rd_u32(n_at + 8), 13);
    }

    #[test]
    fn old_spikes_are_dropped_from_the_ring() {
        let mut s = Shared::default();
        for t in 0..1000u32 {
            s.tick = t;
            s.push_spikes(&[t]);
        }
        // Everything older than the history window must be gone.
        let oldest = s.spikes.front().unwrap().0;
        assert!(
            s.tick - oldest <= SPIKE_HISTORY_TICKS,
            "ring kept {} ticks of history",
            s.tick - oldest
        );
        assert!(!s.spikes.is_empty(), "the ring should still hold recent spikes");
    }

    #[test]
    fn a_response_is_capped_so_one_burst_cannot_flood_the_socket() {
        let mut s = Shared::default();
        s.tick = 10;
        s.group_hz = vec![0.0];
        for i in 0..MAX_SPIKES_PER_RESPONSE as u32 + 5000 {
            s.spikes.push_back((10, i));
        }
        let b = s.encode(0, 1);
        let n_at = 124 + 4;
        let n = u32::from_le_bytes(b[n_at..n_at + 4].try_into().unwrap()) as usize;
        assert_eq!(n, MAX_SPIKES_PER_RESPONSE);
        assert_eq!(b.len(), n_at + 4 + n * 4);
    }
}
