# drosophila

A small 3D fly world with a FlyWire-derived brain network. Rust runs the
simulation; a browser draws a lightweight, articulated fruit fly.

一个尽量简洁的果蝇 3D 世界：趋光、避障、落地、走路、理毛，再次起飞。
完整脑网络保留，画面采用低面数模型和批量绘制，脑活动面板按需打开。

## Run / 运行

Download an archive matching your computer from
[Releases](https://github.com/fuyifu2024/drosophila/releases), unpack it and keep
`data/` beside the executable. No data download is needed after unpacking.

| Computer | Archive |
| --- | --- |
| Linux PC, Intel / AMD 64-bit | linux-x86_64 |
| Linux ARM64 / Raspberry Pi 5 | linux-arm64 |
| Mac with Apple Silicon | macos-arm64 |
| Intel Mac | macos-x86_64 |

On Linux or macOS, open a terminal in the unpacked directory:

```sh
./start.sh
```

On macOS you can also double-click `start.command`. These are unsigned builds;
macOS may ask you to approve opening a downloaded program in Privacy & Security.
The viewer opens in your default browser. Keep the terminal open; **Ctrl+C** stops
the simulation. Closing a browser tab does not stop it.

For a headless machine or to choose another port:

```sh
./drosophila run --seconds 0 --serve 8771
```

Then open `http://127.0.0.1:8771/` on that same computer. The viewer binds only
to loopback. A modern browser with **WebGL 2** is required; Chrome, Firefox,
Safari and Edge are suitable when WebGL 2 is enabled.

**B** opens the optional neuron point cloud. **C** switches between follow,
room overview and side cameras. The on-screen buttons work with a mouse or touch.
**H** hides the buttons. Rendering and polling pause while the page is hidden.

## Build from source / 从源码运行

Install Rust **1.85 or newer**, then:

```sh
git clone https://github.com/fuyifu2024/drosophila.git
cd drosophila
./start.sh
```

Or build and run directly:

```sh
cargo build --release --locked
./target/release/drosophila run --seconds 0 --serve 8770 --open
```

The same source builds natively on Linux x86_64 / ARM64 and macOS Intel /
Apple Silicon. No Wayland, Chromium, GNU `stat`, `fuser`, `pkill` or `bc` is
required. “x86_64” means 64-bit Intel/AMD; 32-bit x86 and Windows are not release
targets for this version.

The viewer files are embedded in the executable. Data lookup tries `data/`
beside the executable, then `./data`, then the source tree as a development
fallback. `--data DIR` overrides discovery.

## What was simplified / 简化内容

- Fly: three body volumes, two eyes, two wings and six articulated legs.
  Shared geometry and instancing reduce the fly to **5 draw calls**, including
  the 18 animated leg segments. Bristles, wing veins, tiny appendages and
  decorative body segments are omitted. The local predecessor used 78 drawable
  objects and 10,334 triangles; this model uses 5 batches and 884 triangles
  (geometry counts, not a frame-rate benchmark).
- World: one floor, a coarse grid, a room outline, three pillars, four food
  patches, twelve pebbles and one lamp. Obstacles use the same dimensions as
  collision detection. Repeated objects are instanced.
- Lighting: ambient plus directional light. No sky shader, fog, shadow maps,
  post-processing or point lights. Pixel ratio stays at one and render rate
  is capped at 60 fps.
- Brain panel: off initially; neuron positions and labels load only when opened.
  It renders points without the legacy anatomy surface meshes.
- Transport: one state request at a time, about 20 Hz, with timeouts and visible
  reconnection status. The physics remains independent of rendering.

These are reductions in scene complexity, not a measured promise of a particular
frame rate on every computer. `window.__drosophila` exposes the current render
counts and connection status for development checks.

## Model boundaries / 模型边界

The bundled network header declares **139,255 neurons**, **2,656,027 edges** and
**33,236,616 synapses**. The simulation uses a simplified LIF neuron model with
a 1 ms step. See [data provenance](data/README.md).

Hard-coded reflex controllers do most of the navigation. Descending-neuron
activity modifies turn and thrust; the network does not independently generate
all the fly's behaviour. Food, grooming and takeoff transitions contain hand-set
rules. The body is a simplified point-and-orientation model, with no muscle or
aerodynamic simulation. There is no synaptic learning. This is an educational
simulation and visualisation, not a validated reproduction of a living fly.

```sh
./target/release/drosophila stats
./target/release/drosophila bench --seconds 2
./target/release/drosophila run --seconds 10 --brain 0
```

`--brain 0` disconnects the network's influence on steering while the network
still runs. `--reflex 0` removes the reflex contribution; stable navigation
should not be expected. `--free-run` removes wall-clock pacing.

## Checks and portable packages

```sh
cargo test --locked
node tests/viewer.test.mjs
cargo build --release --locked
python3 tools/verify_data.py
python3 tools/smoke.py --binary target/release/drosophila
python3 tools/package.py --binary target/release/drosophila --name drosophila-linux-arm64
```

Node.js 22+ and Python 3 are development/test tools only. The smoke test starts
a temporary loopback server from an unrelated working directory, validates
state progression and embedded assets, checks invalid options, and tears it down.
GitHub Actions runs native tests and builds for all four release targets and
retains portable archives. OS-native UI and GPU behaviour still need testing on
real hardware; CI verifies native execution and HTTP, not macOS Gatekeeper or
browser GPU performance.

## Code and data licenses

Project code: [MIT](LICENSE). Bundled three.js: MIT, see
[third-party notices](THIRD_PARTY_NOTICES.md). FlyWire-derived data:
**CC BY-NC 4.0**, with attribution and noncommercial restrictions; the code
license does not relicense the data. Scientific citations and original project
acknowledgements are in [CREDITS.md](CREDITS.md).
