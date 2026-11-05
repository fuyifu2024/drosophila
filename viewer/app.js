import * as THREE from './vendor/three.module.js';
import { BrainView } from './brain_view.js';
import { buildFly } from './model.js';
import { buildWorld } from './world_view.js';
import { flyAxes, decodeState, loadPoints } from './protocol.js';

const canvas = document.getElementById('view');
const closing = document.getElementById('closing');
const help = document.getElementById('help');
const status = document.getElementById('status');
const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'low-power' });
renderer.setPixelRatio(1);
renderer.setClearColor(0x111923, 1);
renderer.autoClear = false;
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(52, 1, 0.05, 900);
const fly = buildFly();
scene.add(fly.group);
const contactShadow = new THREE.Mesh(new THREE.CircleGeometry(1, 12),
  new THREE.MeshBasicMaterial({ color: 0x000000, transparent: true, opacity: 0.35, depthWrite: false }));
scene.add(contactShadow);
const hud = Object.fromEntries(['brain', 'fly', 'fps'].map(k => [k, document.getElementById('h-' + k)]));
const CAMERA_MODES = ['follow', 'overview', 'side'];
let cameraMode = 0;
let brainPanelOpen = false;
let brain = null;
let brainLoading = null;
let world = null;
let displayedTick = -1;
let hudAt = 0;
const state = { lastTick: 0, data: null, receivedAt: 0 };
let pollTimer = null;
let polling = false;

async function json(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.json();
}

async function toggleBrain() {
  brainPanelOpen = !brainPanelOpen;
  document.getElementById('brain-toggle').setAttribute('aria-pressed', String(brainPanelOpen));
  if (!brainPanelOpen) { if (brain) brain.hide(); return; }
  if (!brain && !brainLoading) {
    status.textContent = 'Loading brain…';
    brainLoading = Promise.all([loadPoints('/points.bin'), json('/labels.json')])
      .then(([points, labels]) => { brain = new BrainView(points, [], labels); })
      .catch(error => {
        brainPanelOpen = false;
        document.getElementById('brain-toggle').setAttribute('aria-pressed', 'false');
        status.textContent = 'Brain view unavailable: ' + error.message;
      }).finally(() => { brainLoading = null; });
  }
}

async function poll() {
  if (polling || document.hidden) return;
  polling = true;
  try {
    const response = await fetch('/state?since=' + state.lastTick, { cache: 'no-store', signal: AbortSignal.timeout(2500) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const s = decodeState(await response.arrayBuffer());
    // A restarted simulator has a fresh clock. Request its spikes from zero.
    if (s.tick < state.lastTick) displayedTick = -1;
    state.lastTick = s.tick;
    state.data = s;
    state.receivedAt = performance.now();
    status.textContent = 'Local simulation · connected';
    status.classList.remove('error');
    if (closing.hidden === false) { closing.hidden = true; window.__boot = 'running'; }
  } catch (error) {
    status.textContent = 'Reconnecting… check the simulator terminal';
    status.classList.add('error');
  } finally {
    polling = false;
    clearTimeout(pollTimer);
    if (!document.hidden) pollTimer = setTimeout(poll, 50);
  }
}

async function boot() {
  window.__boot = 'loading world';
  world = await json('/world.json');
  buildWorld(scene, world);
  window.__boot = 'connecting';
  await poll();
  requestAnimationFrame(frame);
}

document.addEventListener('visibilitychange', () => {
  clearTimeout(pollTimer);
  if (!document.hidden) { lastFrame = performance.now(); poll(); }
});
document.getElementById('brain-toggle').addEventListener('click', toggleBrain);
document.getElementById('camera-toggle').addEventListener('click', () => { cameraMode = (cameraMode + 1) % CAMERA_MODES.length; });
window.addEventListener('keydown', e => {
  if (e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  const k = e.key.toLowerCase();
  if (k === 'b') toggleBrain();
  if (k === 'c') cameraMode = (cameraMode + 1) % CAMERA_MODES.length;
  if (k === 'h') help.hidden = !help.hidden;
});


const camTarget = new THREE.Vector3();
const camPos = new THREE.Vector3();
const basis = new THREE.Matrix4();
let camInit = false;

function updateCamera(s, dt) {
  const { forward, left, up } = flyAxes(s.yaw, s.pitch, s.roll);
  const f = new THREE.Vector3(...forward);
  const l = new THREE.Vector3(...left);
  const u = new THREE.Vector3(...up);
  const p = new THREE.Vector3(...s.pos);

  let want;
  if (cameraMode === 0) {
    // Chase from behind, above and well off to one side. Dead astern the fly
    // is one brown ball seen end-on; a three-quarter view is what shows the
    // body, the head and both wings.
    want = p.clone()
      .addScaledVector(f, -3.6)
      .addScaledVector(u, 1.5)
      .addScaledVector(l, -2.0);
    camTarget.copy(p).addScaledVector(f, 0.35);
  } else if (cameraMode === 1) {
    want = new THREE.Vector3(190, -210, 180);
    camTarget.set(0, 0, 45);
  } else {
    // Side-on, close: the view that shows what the legs and wings are doing.
    want = p.clone().addScaledVector(l, -3.2).addScaledVector(u, 0.9).addScaledVector(f, -0.6);
    camTarget.copy(p);
  }

  if (!camInit) {
    camPos.copy(want);
    camInit = true;
  }
  // Smooth the camera rather than welding it to the fly. A camera that tracks
  // a fly exactly is unwatchable: every wobble of the animal becomes a wobble
  // of the whole world.
  // Tight enough that a fly doing 70 mm/s does not outrun the frame. A camera
  // that lags a fast animal pushes it off-centre exactly when there is most to
  // look at.
  const tau = cameraMode === 1 ? 0.14 : 0.065;
  const k = 1 - Math.exp(-dt / tau);
  camPos.lerp(want, k);
  camera.position.copy(camPos);
  camera.up.set(0, 0, 1);
  camera.lookAt(camTarget);
}

let lastFrame = performance.now();
let sizeDirty = true;
window.addEventListener('resize', () => { sizeDirty = true; });

function frame(now) {
  requestAnimationFrame(frame);
  if (document.hidden || now - lastFrame < 1000 / 60 - 1) return;
  const dt = Math.min(0.1, (now - lastFrame) / 1000);
  lastFrame = now;

  fpsFrames++;
  if (now - fpsAt > 500) { fps = fpsFrames * 1000 / (now - fpsAt); fpsFrames = 0; fpsAt = now; }
  const s = state.data;
  if (s) {
    const { forward, left, up } = flyAxes(s.yaw, s.pitch, s.roll);
    // Orientation from the axes themselves rather than from Euler angles, so
    // the drawing cannot disagree with the simulation about which way is up.
    basis.makeBasis(
      new THREE.Vector3(...forward),
      new THREE.Vector3(...left),
      new THREE.Vector3(...up)
    );
    fly.group.quaternion.setFromRotationMatrix(basis);
    fly.group.position.set(...s.pos);

    // Straight down from the fly, to the floor. `z` is the altitude, so the
    // shadow is placed at the same x and y and pinned to z ≈ 0 — the earlier
    // version put the fly's lateral position on the vertical axis and pinned
    // the fly's altitude to the floor, which is a ninety-degree error that
    // looks like a shadow that simply refuses to line up.
    const h = Math.max(0, s.pos[2]);
    contactShadow.position.set(s.pos[0], s.pos[1], 0.05);
    contactShadow.scale.setScalar(1.5 + h * 0.10);
    contactShadow.material.opacity = 0.36 * Math.max(0, 1 - h / 70);

    // Wings. Flap is a rotation about the body's forward axis, sweep about its
    // up axis, and the two are a quarter-cycle apart — that phase offset is
    // what makes the tip trace an ellipse instead of a line.
    //
    // `wingFold` comes from the simulation and is 1 while walking: the wings
    // stop beating and lie back along the abdomen, which is what a fly does
    // when it is on its feet.
    const grooming = s.gait === 2;
    const warming = s.gait === 3;
    const fold = s.wingFold;
    const beat = 1 - fold;
    const flap = Math.sin(s.wing) * beat;
    const sweep = Math.sin(s.wing + 1.2) * 0.34 * beat;
    for (const w of fly.wings) {
      // Folding is a rotation about z, sweeping the wing back over the
      // abdomen. It is emphatically *not* a rotation about y: the wing points
      // along y, and rotating a vector about its own axis does nothing at all
      // — which is what the first version did, leaving the wings sticking
      // straight out while the fly was supposedly walking.
      w.pivot.rotation.x = (flap + fold * 0.18) * w.side;
      // 1.35 rad is about 77°: folded wings lie back along the abdomen, not
      // stuck out at forty-five degrees like a parked aircraft.
      w.pivot.rotation.z = (sweep + fold * 1.35) * w.side;
    }

    // --- legs ---
    // Tripod gait: front-left, middle-right and rear-left step together while
    // the other three carry the weight, then they swap. Two things fall out of
    // that pattern for free — at any moment at least three feet are down, so a
    // six-legged walk looks stable, and the body does not bob, so it looks
    // deliberate.
    const phase = s.legPhase * Math.PI * 2;
    const rub = Math.sin(s.groomPhase * Math.PI * 2);
    // Tuck is continuous, not a flag. Switching it at a threshold makes the
    // legs snap between two poses in one frame every time the fly lands or
    // takes off, which is the single most artificial thing the model does.
    const tuck = 1 - fold;

    for (const leg of fly.legs) {
      const onFirstTripod = (leg.index === 1) === (leg.side > 0);
      const p = phase + (onFirstTripod ? 0 : Math.PI);
      const swing = Math.sin(p);
      const lift = Math.min(1, Math.max(0, Math.sin(p)) * 1.7);

      // --- on the ground ---
      let gx, gz, gkx, gtx;
      if (warming) {
        // Coiled for the jump: feet planted, legs compressed by the same
        // amount the body has dropped, no stepping at all. It is the only
        // pose in the whole animation where nothing is moving.
        gx = leg.crouchX;
        gz = 0;
        gkx = leg.crouchKnee;
        gtx = leg.crouchTarsus;
      } else if (grooming && leg.index === 0) {
        // Front legs up to the face, rubbing. This is the whole animation: the
        // other four feet stay planted and the body does not move, so all of
        // the motion in the frame is in these two legs.
        // These four numbers are *solved*, not tuned. The pose is the one that
        // puts the tip of the tarsus on the surface of the head ellipsoid
        // (value 1.001 against a threshold of 1.0) at the front of the face,
        // and outside the eye (3.40), found by coordinate descent against the
        // head and eye volumes. Both earlier attempts were wrong in different
        // directions: positive `gz` put the foot over the back, through the
        // folded wings, and the corrected sign still buried it 0.7 mm inside
        // the head because the leg was too short to reach the face properly.
        //
        // The rub amplitude sweeps the foot along the face rather than into
        // it — at both extremes of the cycle the tarsus stays outside the head
        // (1.374 and 1.091).
        gx = 2.085 + rub * 0.16;
        gz = -1.165 - rub * 0.20;
        gkx = -0.370 + rub * 0.30;
        gtx = 1.395 + rub * 0.45;
      } else if (grooming) {
        // Planted, with the faintest sway, so it does not look switched off.
        const sway = Math.sin(s.groomPhase * Math.PI * 2 + leg.index) * 0.04;
        gx = leg.baseX + sway;
        gz = -sway;
        gkx = leg.baseKnee;
        gtx = leg.baseTarsus;
      } else {
        gx = leg.baseX + lift * 0.38;
        gz = -swing * 0.34;
        gkx = leg.baseKnee - lift * 0.5;
        gtx = leg.baseTarsus + lift * 0.3;
      }

      // --- airborne: folded back and up, out of the airflow ---
      const tx = leg.baseX + 1.75;
      const tz = 0.65;
      const tkx = leg.baseKnee - 0.55;
      const ttx = leg.baseTarsus - 0.9;

      // x rotations mirrored by `side`, z rotations mirrored by `side` in
      // their own expressions. See `buildLeg` for why.
      const mix = (a, b) => (a * (1 - tuck) + b * tuck) * leg.side;
      leg.root.rotation.x = mix(gx, tx);
      leg.root.rotation.z = mix(gz, tz);
      leg.knee.rotation.x = mix(gkx, tkx);
      leg.tarsus.rotation.x = mix(gtx, ttx);
    }

    fly.syncLegs();
    updateCamera(s, dt);

    if (brain && brainPanelOpen && s.tick !== displayedTick) {
      brain.update(s.spikes, s.tick, s.groupHz);
      displayedTick = s.tick;
    }
    if (now - hudAt > 250) { updateHud(s); hudAt = now; }
  }

  // Reading clientWidth every frame forces a style recalculation, which is not
  // free on this machine. The window only changes size when it does.
  if (sizeDirty) {
    sizeDirty = false;
    const w = canvas.clientWidth || window.innerWidth;
    const h = canvas.clientHeight || window.innerHeight;
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  }
  const w = renderer.domElement.width;
  const h = renderer.domElement.height;

  renderer.setScissorTest(false);
  renderer.setViewport(0, 0, w, h);
  renderer.clear(true, true, false);
  renderer.render(scene, camera);
  const worldCalls = renderer.info.render.calls;
  const worldTriangles = renderer.info.render.triangles;

  if (brain) {
    if (brainPanelOpen) {
      brain.render(renderer, w, h);
    } else {
      brain.hide();
    }
  }
  window.__drosophila = { tick: state.lastTick, worldCalls, worldTriangles, brainOpen: brainPanelOpen, modelCalls: fly.drawCalls, connected: performance.now() - state.receivedAt < 3000 };
}


let fpsFrames = 0;
let fpsAt = performance.now();
let fps = 0;
function updateHud(s) {
  const names = ['Flying', 'Walking', 'Grooming', 'Taking off'];
  hud.fly.textContent = `${names[s.gait] || 'Unknown'} · ${s.speed.toFixed(0)} mm/s · z ${s.pos[2].toFixed(1)} mm`;
  hud.brain.textContent = `139,255 neurons · ${(s.tick / 1000).toFixed(1)} s`;
  hud.fps.textContent = `${fps.toFixed(0)} fps · ${CAMERA_MODES[cameraMode]}`;
}

boot().catch(error => {
  window.__boot = 'failed';
  closing.hidden = false;
  closing.textContent = 'Could not start: ' + error.message + '. Check the simulator terminal and reload.';
});
