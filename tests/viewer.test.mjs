import test from 'node:test';
import assert from 'node:assert/strict';
import * as THREE from '../viewer/vendor/three.module.js';
import { flyAxes, decodeState } from '../viewer/protocol.js';
import { buildFly } from '../viewer/model.js';
import { buildWorld } from '../viewer/world_view.js';

test('body axes stay orthonormal and right handed through pitch and roll', () => {
  for (const yaw of [0, 0.7, -2.1]) for (const pitch of [-0.5, 0, 1.2]) for (const roll of [-1, 0.3]) {
    const a = flyAxes(yaw, pitch, roll);
    const f = new THREE.Vector3(...a.forward), l = new THREE.Vector3(...a.left), u = new THREE.Vector3(...a.up);
    assert.ok(Math.abs(f.length() - 1) < 1e-10);
    assert.ok(Math.abs(l.length() - 1) < 1e-10);
    assert.ok(Math.abs(f.dot(l)) < 1e-10);
    assert.ok(f.clone().cross(l).distanceTo(u) < 1e-10);
  }
});

test('the state decoder retains gait, groups and spike indices', () => {
  const buffer = new ArrayBuffer(144);
  const view = new DataView(buffer);
  view.setUint32(0, 0x41545344, true);
  view.setUint32(4, 1234, true);
  view.setFloat32(8, 12.5, true);
  view.setUint32(92, 2, true);
  view.setUint32(120, 2, true);
  view.setFloat32(124, 1.25, true);
  view.setFloat32(128, 2.5, true);
  view.setUint32(132, 2, true);
  view.setUint32(136, 17, true);
  view.setUint32(140, 130000, true);
  const state = decodeState(buffer);
  assert.equal(state.tick, 1234);
  assert.equal(state.pos[0], 12.5);
  assert.equal(state.gait, 2);
  assert.deepEqual([...state.groupHz], [1.25, 2.5]);
  assert.deepEqual([...state.spikes], [17, 130000]);
  assert.throws(() => decodeState(buffer.slice(0, 127)), /truncated/);
  view.setUint32(132, 10000, true);
  assert.throws(() => decodeState(buffer), /truncated spikes/);
});

test('batched leg segments follow the rig without baking the body translation twice', () => {
  const fly = buildFly();
  let calls = 0;
  fly.group.traverse(node => { if (node.isMesh) calls++; });
  assert.equal(calls, 5);
  const legs = fly.group.children.find(node => node.isInstancedMesh && node.count === 18);
  const initial = new THREE.Matrix4();
  legs.getMatrixAt(0, initial);
  fly.group.position.set(45, -20, 12);
  fly.group.rotation.set(0.3, -0.2, 0.8);
  fly.syncLegs();
  const translated = new THREE.Matrix4();
  legs.getMatrixAt(0, translated);
  translated.elements.forEach((value, i) => assert.ok(Math.abs(value - initial.elements[i]) < 1e-6));
  fly.legs[0].root.rotation.z += 0.5;
  fly.syncLegs();
  legs.getMatrixAt(0, translated);
  assert.notDeepEqual(translated.elements, initial.elements);
});

test('the rest pose puts all six feet on the floor', () => {
  const fly = buildFly();
  fly.group.position.z = 0.55;
  fly.group.updateMatrixWorld(true);
  for (const leg of fly.legs) {
    const segment = leg.tarsus.children[0];
    const foot = new THREE.Vector3(0, leg.side * 0.5, 0).applyMatrix4(segment.matrixWorld);
    assert.ok(Math.abs(foot.z) < 1e-6, `foot above/below floor: ${foot.z}`);
  }
});

test('world instances use the simulator dimensions and z-up ground', () => {
  const scene = new THREE.Scene();
  buildWorld(scene, { half_xy: 100, height: 120, pillars: [{ x: 55, y: 0, r: 7 }], food: [[10, 20]], food_radius: 5, pebbles: [[-10, 7, 1.2]], light: [0, 0, 95] });
  const pillars = scene.children.find(node => node.isInstancedMesh && node.geometry.type === 'CylinderGeometry');
  const m = new THREE.Matrix4();
  pillars.getMatrixAt(0, m);
  const top = new THREE.Vector3(0, 0.5, 0).applyMatrix4(m);
  assert.ok(Math.abs(top.x - 55) < 1e-5);
  assert.ok(Math.abs(top.y) < 1e-5);
  assert.ok(Math.abs(top.z - 120) < 1e-5);
});
