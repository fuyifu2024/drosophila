// The corner panel: the fly's brain, lit by its own spikes.
//
// # Why there are three colours and not twenty-one
//
// The connectome carries twenty-one named cell groups and it is tempting to
// give each one a hue. That is exactly the mistake. A point cloud is an
// all-pairs form — every group can end up next to every other — and under
// all-pairs separation only three categorical hues clear the colourblind and
// normal-vision gates. Past that the pairs stop being distinguishable and the
// legend becomes decoration.
//
// So the twenty-one fold into three, chosen because they are the three things
// the panel is actually trying to say:
//
//   * **input**  — photoreceptors, the T4/T5 motion detectors, LPLC2/LC4.
//                  What the world is doing to the fly.
//   * **output** — descending and motor neurons, the brain's only way out.
//                  What the fly is doing about it.
//   * **intrinsic** — Kenyon cells and the central complex. Memory and
//                  navigation: the two structures a reader is most likely to
//                  have heard of.
//
// Everything else — 118,000 neurons of it — is a recessive slate. That is not
// a demotion. The story this panel tells is *input goes in, output comes out,
// and everything between them is the connectome*, and a fourth, fifth and
// sixth hue would only obscure it.
//
// Activity is carried by brightness *within* each hue, never by changing hue,
// so a flashing orange neuron is still an output neuron.

import * as THREE from './vendor/three.module.js';

// Matches `etl.rs::GROUP_NAMES` order.
const G_PHOTORECEPTOR = 1;
const G_T4A = 2, G_T5D = 9;
const G_LPLC2 = 10, G_LC4 = 11;
const G_DESCENDING = 12, G_MOTOR = 13;
const G_KENYON = 14, G_CENTRAL_COMPLEX = 15;

const CAT_INPUT = 0;
const CAT_OUTPUT = 1;
const CAT_INTRINSIC = 2;
const CAT_OTHER = 3;

export const CATEGORIES = [
  { key: 'input', label: 'input', color: 0x3987e5, note: 'photoreceptors, T4/T5, LPLC2/LC4' },
  { key: 'output', label: 'output', color: 0xd95926, note: 'descending + motor neurons' },
  { key: 'intrinsic', label: 'intrinsic', color: 0x199e70, note: 'mushroom body, central complex' },
  { key: 'other', label: 'other', color: 0x3a4656, note: 'the rest of the connectome' },
];

/// The two ends of every hue: what a neuron looks like at rest and what it
/// looks like mid-spike. Activity moves a point along its own hue's ramp and
/// never to a different hue, so a flashing orange neuron is still an output
/// neuron.
function palette() {
  const dim = [];
  const hot = [];
  for (const c of CATEGORIES) {
    const col = new THREE.Color(c.color);
    hot.push(col.r, col.g, col.b);
    dim.push(col.r * 0.34, col.g * 0.34, col.b * 0.34);
  }
  return { dim, hot };
}

function categoryOf(g) {
  if (g >= G_PHOTORECEPTOR && g <= G_LC4) return CAT_INPUT;
  if (g === G_DESCENDING || g === G_MOTOR) return CAT_OUTPUT;
  if (g === G_KENYON || g === G_CENTRAL_COMPLEX) return CAT_INTRINSIC;
  return CAT_OTHER;
}

/// Texture and attribute sizing: 512 wide keeps every index reachable and the
/// stride a power of two, which is friendlier to the upload path.

const VERT = `
  attribute float aCat;
  attribute float aAct;
  uniform vec3 uDim[4];
  uniform vec3 uHot[4];
  uniform float uSize;
  varying vec3 vColor;
  varying float vAct;
  void main() {
    int c = int(aCat + 0.5);
    vAct = aAct;
    vColor = mix(uDim[c], uHot[c], aAct);
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
    // Perspective sizing, but clamped: at this scale the clamp is what keeps
    // a neuron from becoming a blob when the camera comes close.
    gl_PointSize = clamp(uSize * (1.4 / -mv.z), 1.0, 5.0);
  }
`;

const FRAG = `
  uniform float uAlpha;
  varying vec3 vColor;
  varying float vAct;
  void main() {
    // Round points, and discard the corners rather than blending them, so
    // 139,000 overlapping sprites do not turn the panel into fog.
    vec2 d = gl_PointCoord - 0.5;
    if (dot(d, d) > 0.25) discard;
    vec3 c = vColor + vAct * vAct * 0.55;
    gl_FragColor = vec4(c, uAlpha);
  }
`;

export class BrainView {
  constructor(points, meshes, labels) {
    this.points = points;
    this.labels = labels;
    this.n = points.n;
    this.tick = 0;
    this.decayCountdown = 0;
    this.spin = 0;

    this.activity = new Float32Array(this.n);

    this.scene = new THREE.Scene();
    this.camera = new THREE.PerspectiveCamera(38, 1, 0.02, 12);

    this.buildCloud();
    this.buildLegend();
  }

  buildCloud() {
    const { pos, group } = this.points;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(pos, 3));

    const cat = new Float32Array(this.n);
    for (let i = 0; i < this.n; i++) cat[i] = categoryOf(group[i]);
    geo.setAttribute('aCat', new THREE.BufferAttribute(cat, 1));
    geo.setAttribute('aAct', new THREE.BufferAttribute(this.activity, 1));

    const { dim, hot } = palette();
    const mat = new THREE.ShaderMaterial({
      uniforms: {
        uDim: { value: dim },
        uHot: { value: hot },
        uSize: { value: 1.9 },
        uAlpha: { value: 0.92 },
      },
      vertexShader: VERT,
      fragmentShader: FRAG,
      transparent: true,
      depthWrite: false,
      blending: THREE.NormalBlending,
    });

    this.cloudMat = mat;
    this.cloud = new THREE.Points(geo, mat);
    this.scene.add(this.cloud);
    this.geo = geo;
  }

  buildLegend() {
    // A title, because the panel is a cloud of dots until somebody says what
    // it is. The legend below it explains the colours; this says what the
    // colours are *of*.
    const title = document.createElement('div');
    title.id = 'braintitle';
    title.style.cssText = [
      'position:fixed', 'pointer-events:none',
      'font:10px/1.5 "DejaVu Sans Mono",monospace',
      'text-shadow:0 1px 3px #000', 'color:#96a4b6', 'z-index:5',
    ].join(';');
    // Short enough not to wrap in the 240 px the panel gets on an 800 px
    // screen. The first version ran onto two lines and collided with the
    // legend underneath it.
    title.textContent = 'brain · 139,255 neurons, live';
    document.body.appendChild(title);
    this.titleEl = title;

    const el = document.createElement('div');
    el.id = 'brainlegend';
    el.style.cssText = [
      'position:fixed', 'pointer-events:none', 'font:10px/1.5 "DejaVu Sans Mono",monospace',
      'text-shadow:0 1px 3px #000', 'color:#7c8899', 'z-index:5',
    ].join(';');
    // One row, not four. The 800x480 screen gives the panel 240 px of width,
    // and four captioned rows need more than that — the first version ran off
    // the bottom of the screen.
    el.style.whiteSpace = 'nowrap';
    for (const c of CATEGORIES) {
      const hex = '#' + c.color.toString(16).padStart(6, '0');
      const item = document.createElement('span');
      item.style.marginRight = '9px';
      item.innerHTML =
        '<span style="display:inline-block;width:7px;height:7px;background:' + hex +
        ';margin-right:4px;vertical-align:middle"></span>' +
        '<span style="color:#cfd6e0">' + c.label + '</span>';
      item.title = c.note;
      el.appendChild(item);
    }
    document.body.appendChild(el);
    this.legendEl = el;
  }

  /// Fold a spike train into the activity field. Brightness is set, not added,
  /// so a neuron firing at 200 Hz looks the same as one firing at 20 — this is
  /// a picture of *where* the brain is busy, not how busy.
  update(spikes, tick, groupHz) {
    this.tick = tick;
    this.lastGroups = groupHz;
    for (let i = 0; i < spikes.length; i++) {
      const n = spikes[i];
      if (n < this.n) this.activity[n] = 1.0;
    }
    const elapsed = Math.max(0, tick - (this.lastDecayTick ?? tick));
    const decay = Math.exp(-elapsed / 180);
    for (let i = 0; i < this.n; i++) this.activity[i] *= decay;
    this.lastDecayTick = tick;
    this.geo.attributes.aAct.needsUpdate = true;
  }

  render(renderer, w, h) {
    const pw = Math.min(w - 12, Math.max(240, Math.round(w * 0.30)));
    const ph = Math.min(h - 100, Math.max(110, Math.round(h * 0.38)));
    const margin = 6;
    const px = w - pw - margin;
    const py = margin; // WebGL's origin is bottom-left

    this.spin = this.tick * 0.00015;
    const r = 0.78;
    this.camera.position.set(
      Math.cos(this.spin) * r,
      Math.sin(this.spin * 0.7) * r * 0.5,
      Math.sin(this.spin) * r * 0.9
    );
    this.camera.up.set(0, 0, 1);
    this.camera.lookAt(0, 0, 0);
    this.camera.aspect = pw / ph;
    this.camera.updateProjectionMatrix();

    renderer.setScissorTest(true);
    renderer.setViewport(px, py, pw, ph);
    renderer.setScissor(px, py, pw, ph);
    renderer.setClearColor(0x080d14, 1);
    renderer.clear(true, true, false);
    renderer.render(this.scene, this.camera);
    renderer.setScissorTest(false);
    renderer.setClearColor(0x111923, 1);

    // The legend is DOM, so it has to be placed against the same rectangle.
    // CSS is top-left origin, WebGL is bottom-left.
    const top = h - py - ph;
    this.titleEl.style.left = px + 6 + 'px';
    this.titleEl.style.top = top - 27 + 'px';
    this.titleEl.style.display = '';
    this.legendEl.style.left = px + 6 + 'px';
    this.legendEl.style.top = top - 14 + 'px'; // just above the panel
    this.legendEl.style.display = '';
  }

  hide() {
    this.legendEl.style.display = 'none';
    this.titleEl.style.display = 'none';
  }
}
