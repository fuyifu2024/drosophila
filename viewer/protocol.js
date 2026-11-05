// Little-endian simulation protocol. Body axes mirror src/body.rs.
const FILE_MAGIC = 0x4e494252;
const STATE_MAGIC = 0x41545344;

export function flyAxes(yaw, pitch, roll) {
  const sy = Math.sin(yaw), cy = Math.cos(yaw);
  const sp = Math.sin(pitch), cp = Math.cos(pitch);
  const sr = Math.sin(roll), cr = Math.cos(roll);
  const forward = [cp * cy, cp * sy, sp];
  const left = [-cy * sp * sr - sy * cr, cy * cr - sy * sp * sr, cp * sr];
  const up = [-cy * sp * cr + sy * sr, -sy * sp * cr - cy * sr, cp * cr];
  return { forward, left, up };
}

// --- binary loaders ---------------------------------------------------------

export async function loadPoints(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  const buf = await response.arrayBuffer();
  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== FILE_MAGIC) throw new Error('points.bin: bad magic');
  const n = dv.getUint32(4, true);
  const nClasses = dv.getUint32(8, true);
  const nGroups = dv.getUint32(12, true);
  // The sections are laid out so every typed-array view lands on a 4-byte
  // boundary: 16 bytes of header, then n*12 of floats.
  let o = 16;
  const pos = new Float32Array(buf, o, n * 3); o += n * 12;
  const cls = new Uint8Array(buf, o, n); o += n;
  const side = new Uint8Array(buf, o, n); o += n;
  const group = new Uint8Array(buf, o, n); o += n;
  return { n, nClasses, nGroups, pos, cls, side, group };
}

// --- state decoding ---------------------------------------------------------

export function decodeState(buf) {
  const dv = new DataView(buf);
  if (buf.byteLength < 128) throw new Error('state: truncated header');
  if (dv.getUint32(0, true) !== STATE_MAGIC) throw new Error('state: bad magic');
  const s = {
    tick: dv.getUint32(4, true),
    pos: [dv.getFloat32(8, true), dv.getFloat32(12, true), dv.getFloat32(16, true)],
    yaw: dv.getFloat32(20, true),
    pitch: dv.getFloat32(24, true),
    roll: dv.getFloat32(28, true),
    wing: dv.getFloat32(32, true),
    wingPower: dv.getFloat32(36, true),
    speed: dv.getFloat32(40, true),
    bumps: dv.getUint32(44, true),
    eyeL: dv.getFloat32(48, true),
    eyeR: dv.getFloat32(52, true),
    motion: [0, 1, 2, 3].map((k) => dv.getFloat32(56 + k * 4, true)),
    loom: dv.getFloat32(72, true),
    dnL: dv.getFloat32(76, true),
    dnR: dv.getFloat32(80, true),
    yawCmd: dv.getFloat32(84, true),
    thrust: dv.getFloat32(88, true),
    gait: dv.getUint32(92, true), // 0 flying, 1 walking, 2 grooming, 3 warming
    legPhase: dv.getFloat32(96, true),
    groomPhase: dv.getFloat32(100, true),
    wingFold: dv.getFloat32(104, true),
    walkedMm: dv.getFloat32(108, true),
    gaitChanges: dv.getUint32(112, true),
    worldT: dv.getFloat32(116, true),
  };
  const g = dv.getUint32(120, true);
  if (g > (buf.byteLength - 128) / 4) throw new Error('state: truncated groups');
  s.groupHz = new Float32Array(g);
  for (let k = 0; k < g; k++) s.groupHz[k] = dv.getFloat32(124 + k * 4, true);
  let o = 124 + g * 4;
  const n = dv.getUint32(o, true); o += 4;
  if (n > (buf.byteLength - o) / 4) throw new Error('state: truncated spikes');
  s.spikes = new Uint32Array(buf, o, n);
  return s;
}
