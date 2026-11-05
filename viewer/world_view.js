// Every solid obstacle is supplied by the simulator; z is up throughout.
import * as THREE from './vendor/three.module.js';

export function buildWorld(scene, world) {
  scene.background = new THREE.Color(0x111923);
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(world.half_xy * 2, world.half_xy * 2),
    new THREE.MeshLambertMaterial({ color: 0x283744 }));
  scene.add(floor);
  const grid = new THREE.GridHelper(world.half_xy * 2, 10, 0x4e6573, 0x344752);
  grid.rotation.x = Math.PI / 2;
  grid.position.z = 0.02;
  scene.add(grid);
  const box = new THREE.BoxGeometry(world.half_xy * 2, world.half_xy * 2, world.height);
  const outline = new THREE.LineSegments(new THREE.EdgesGeometry(box), new THREE.LineBasicMaterial({ color: 0x526879 }));
  box.dispose();
  outline.position.z = world.height / 2;
  scene.add(outline);

  function batch(geometry, color, count, populate) {
    if (!count) { geometry.dispose(); return; }
    const mesh = new THREE.InstancedMesh(geometry, new THREE.MeshLambertMaterial({ color }), count);
    const part = new THREE.Object3D();
    for (let i = 0; i < count; i++) {
      populate(part, i);
      part.updateMatrix();
      mesh.setMatrixAt(i, part.matrix);
    }
    mesh.instanceMatrix.needsUpdate = true;
    scene.add(mesh);
  }
  batch(new THREE.CylinderGeometry(1, 1, 1, 8), 0x58717e, world.pillars.length, (part, i) => {
    const p = world.pillars[i];
    part.position.set(p.x, p.y, world.height / 2);
    part.rotation.x = Math.PI / 2;
    part.scale.set(p.r, world.height, p.r);
  });
  batch(new THREE.CircleGeometry(1, 12), 0xc6984c, world.food.length, (part, i) => {
    part.position.set(...world.food[i], 0.04);
    part.scale.setScalar(world.food_radius);
  });
  batch(new THREE.SphereGeometry(1, 6, 4), 0x647783, world.pebbles.length, (part, i) => {
    const [x, y, r] = world.pebbles[i];
    part.position.set(x, y, r);
    part.scale.setScalar(r);
  });
  const lamp = new THREE.Mesh(new THREE.OctahedronGeometry(3), new THREE.MeshBasicMaterial({ color: 0xffd57d }));
  lamp.position.set(...world.light);
  scene.add(lamp);
  // Two simple lights; no shadow maps, point lights, sky shaders or fog.
  scene.add(new THREE.AmbientLight(0xe4edf4, 2.0));
  const sun = new THREE.DirectionalLight(0xffedcd, 2.0);
  sun.position.set(-50, -30, 120);
  scene.add(sun);
}
