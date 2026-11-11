// Smooth snapshots without changing the simulation clock or its protocol.
const clamp = (x, lo, hi) => Math.min(hi, Math.max(lo, x));
const mix = (a, b, t) => a + (b - a) * t;
const angle = (a, b, t) => a + Math.atan2(Math.sin(b - a), Math.cos(b - a)) * t;

export function interpolate(previous, current, elapsedMs, intervalMs) {
  if (!previous || current.tick <= previous.tick) return current;
  const t = clamp(elapsedMs / clamp(intervalMs, 16, 150), 0, 1);
  return { ...current,
    pos: current.pos.map((v, i) => mix(previous.pos[i], v, t)),
    yaw: angle(previous.yaw, current.yaw, t),
    pitch: angle(previous.pitch, current.pitch, t),
    roll: angle(previous.roll, current.roll, t),
    wingFold: mix(previous.wingFold, current.wingFold, t),
    // The transmitted 18 Hz phase aliases at a 20 Hz polling rate. Animate
    // from the latest phase instead of interpolating through the short arc.
    wing: current.wing + Math.min(Math.max(elapsedMs, 0), 100) * 0.001 * 18 * Math.PI * 2,
  };
}

// A set is bounded by the neuron count and preserves all active neuron IDs
// between browser frames. The panel visualizes activity, not spike counts.
export class PendingSpikes {
  constructor() { this.ids = new Set(); }
  add(spikes) { for (const id of spikes) this.ids.add(id); }
  clear() { this.ids.clear(); }
  take() { const result = Uint32Array.from(this.ids); this.clear(); return result; }
}
