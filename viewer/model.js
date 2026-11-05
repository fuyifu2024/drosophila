// A small articulated fly. Static parts and the 18 leg segments are instanced;
// joint transforms still drive walking, grooming, folding and crouching.
import * as THREE from './vendor/three.module.js';

const STAND_HEIGHT = 0.55; // src/body.rs
const CROUCH_DEPTH = 0.82;
const LEG_SPEC = [
  { hip: [0.38, 0.24, -0.16], femur: 0.26, tibia: 0.21, tarsus: 0.14, spread: 0.20 },
  { hip: [-0.02, 0.23, -0.17], femur: 0.26, tibia: 0.20, tarsus: 0.13, spread: 0.24 },
  { hip: [-0.34, 0.21, -0.12], femur: 0.30, tibia: 0.23, tarsus: 0.15, spread: 0.26 },
];

export function solveLeg(femur, tibia, tarsus, spread, drop) {
  const l2 = tibia + tarsus;
  const d = Math.min(Math.max(Math.hypot(spread, drop), Math.abs(femur - l2) + 1e-4), femur + l2 - 1e-4);
  return {
    femur: Math.atan2(-drop, spread) + Math.acos((femur * femur + d * d - l2 * l2) / (2 * femur * d)),
    knee: Math.acos((femur * femur + l2 * l2 - d * d) / (2 * femur * l2)) - Math.PI,
    tarsus: 0,
  };
}

export function buildFly() {
  const group = new THREE.Group();
  const sphere = new THREE.SphereGeometry(1, 10, 6);
  const tmp = new THREE.Object3D();
  function instances(parts, color) {
    const batch = new THREE.InstancedMesh(sphere, new THREE.MeshLambertMaterial({ color, flatShading: true }), parts.length);
    parts.forEach(([position, scale], i) => {
      tmp.position.set(...position);
      tmp.scale.set(...scale);
      tmp.updateMatrix();
      batch.setMatrixAt(i, tmp.matrix);
    });
    batch.instanceMatrix.needsUpdate = true;
    group.add(batch);
  }
  // Head, thorax, abdomen; the red eyes carry the recognisable silhouette.
  instances([
    [[0.62, 0, 0.02], [0.26, 0.31, 0.28]],
    [[0.02, 0, 0.01], [0.42, 0.38, 0.37]],
    [[-0.70, 0, -0.02], [0.62, 0.34, 0.31]],
  ], 0xb18a50);
  instances([1, -1].map(s => [[0.65, s * 0.23, 0.07], [0.14, 0.18, 0.21]]), 0xa83c32);

  const legBatch = new THREE.InstancedMesh(
    new THREE.CylinderGeometry(1, 0.65, 1, 5),
    new THREE.MeshLambertMaterial({ color: 0x463b32 }), 18
  );
  legBatch.instanceMatrix.setUsage(THREE.DynamicDrawUsage);
  // Pose changes can leave the initial bounds. The fly is tiny: skip culling
  // this one batch instead of recomputing an enclosing sphere every frame.
  legBatch.frustumCulled = false;
  group.add(legBatch);
  const segments = [];
  const legs = [];
  for (const side of [1, -1]) {
    LEG_SPEC.forEach((spec, index) => {
      const root = new THREE.Group();
      root.position.set(spec.hip[0], side * spec.hip[1], spec.hip[2]);
      group.add(root);
      const knee = new THREE.Group();
      knee.position.y = side * spec.femur;
      root.add(knee);
      const tarsus = new THREE.Group();
      tarsus.position.y = side * spec.tibia;
      knee.add(tarsus);
      for (const [joint, length, radius] of [[root, spec.femur, 0.03], [knee, spec.tibia, 0.022], [tarsus, spec.tarsus, 0.013]]) {
        const segment = new THREE.Object3D();
        segment.position.y = side * length / 2;
        segment.scale.set(radius, length, radius);
        joint.add(segment);
        segments.push(segment);
      }
      const pose = solveLeg(spec.femur, spec.tibia, spec.tarsus, spec.spread, spec.hip[2] + STAND_HEIGHT);
      const crouch = solveLeg(spec.femur, spec.tibia, spec.tarsus, spec.spread, spec.hip[2] + STAND_HEIGHT * CROUCH_DEPTH);
      root.rotation.x = pose.femur * side;
      knee.rotation.x = pose.knee * side;
      legs.push({ root, knee, tarsus, side, index, baseX: pose.femur, baseKnee: pose.knee, baseTarsus: 0,
        crouchX: crouch.femur, crouchKnee: crouch.knee, crouchTarsus: 0 });
    });
  }
  const wingGeo = new THREE.CircleGeometry(1, 12);
  const wingMat = new THREE.MeshBasicMaterial({ color: 0xc2dcea, transparent: true, opacity: 0.5, side: THREE.DoubleSide, depthWrite: false, forceSinglePass: true });
  const wings = [1, -1].map(side => {
    const pivot = new THREE.Group();
    pivot.position.set(-0.04, side * 0.16, 0.26);
    const wing = new THREE.Mesh(wingGeo, wingMat);
    wing.position.set(-0.14, side * 0.88, 0);
    wing.scale.set(0.30, 0.92, 1);
    pivot.add(wing);
    group.add(pivot);
    return { pivot, side };
  });
  const inverse = new THREE.Matrix4();
  const transform = new THREE.Matrix4();
  function syncLegs() {
    group.updateMatrixWorld(true);
    inverse.copy(group.matrixWorld).invert();
    segments.forEach((part, i) => {
      transform.multiplyMatrices(inverse, part.matrixWorld);
      legBatch.setMatrixAt(i, transform);
    });
    legBatch.instanceMatrix.needsUpdate = true;
  }
  syncLegs();
  return { group, wings, legs, syncLegs, drawCalls: 5 };
}
