'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const M = require('./model.js');
const p = M.validate(M.defaults);
const near = (a, b, tol = 1e-8) => assert.ok(Math.abs(a - b) <= tol, `${a} != ${b}`);

test('fixed-turn radii, three wheel positions and side-wheel scrub are asymmetric', () => {
  const L = 1.5, a = 0.3, d = 0.9;
  near(Math.atan(L * 0.25) / M.RAD, 20.556045219583467);
  near(Math.hypot(4, L), 4.272001872658765);
  near(Math.hypot(4 + d, a), 4.909175083453431);
  near(Math.hypot(4 - d, a), 3.1144823004794873);
  near(Math.atan2(0.25 * a, 1 + 0.25 * d) / M.RAD, 3.503531644784459, 1e-6);
  near(Math.atan2(-0.25 * a, 1 - 0.25 * d) / M.RAD, -5.527540151656171, 1e-6);
  const pose = { x: 4 * Math.sin(0.5), y: 4 * (1 - Math.cos(0.5)), theta: 0.5 };
  near(pose.x, 1.917702154416812); near(pose.y, 0.489669752438509);
  const front = M.transform(pose, L, 0);
  near(Math.hypot(front.x, front.y - 4), Math.hypot(4, L));
  const side = M.transform(pose, a, -d);
  near(Math.hypot(side.x, side.y - 4), Math.hypot(4 + d, a));
});

test('signed rectangle and capsule distances distinguish tangency, penetration and gaps', () => {
  const rect = { kind: 'rect', x: 1, y: -2, halfX: 1, halfY: 0.5, angle: Math.PI / 2 };
  near(M.poleDistance(rect, { x: 1.5, y: -2 }), 0);
  near(M.poleDistance(rect, { x: 1, y: -2 }), -0.5);
  near(M.poleDistance(rect, { x: 1.6, y: -2 }), 0.1);
  const capsule = { kind: 'capsule', a: { x: 0, y: -1 }, b: { x: 0, y: 1 }, radius: 0.035 };
  near(M.poleDistance(capsule, { x: 0.035, y: 0 }), 0);
  near(M.poleDistance(capsule, { x: 0, y: 0 }), -0.035);
  const pose = { x: 7, y: 0, theta: 0, steer: 0, handle: 0 };
  // Ahead of the front connecting strut is real empty space between the bodies.
  const check = M.inspectPose(pose, { ...p, poleCount: 1 }, { ...M.course(p), poles: [{ x: 8.3, y: -0.55, index: 0 }] });
  assert.ok(check.pole > 0, 'Do not fill the entire vehicle bounding box');
});

test('rotating handle tips and a single corner crossing a boundary are checked', () => {
  const pose = { x: 7, y: 0, theta: 0, steer: 0.7, handle: 0.7 };
  const bar = M.geometry(pose, p).find(s => s.name === '车把端');
  near(Math.hypot(bar.a.x - bar.b.x, bar.a.y - bar.b.y), p.handleWidth);
  const c = { ...M.course(p), poles: [{ ...bar.a, index: 0 }] };
  const collision = M.inspectPose(pose, { ...p, poleCount: 1 }, c);
  assert.ok(collision.pole <= -p.poleRadius); assert.equal(collision.polePart, '车把端');
  const rotated = { ...pose, theta: Math.PI / 4, y: p.laneWidth / 2 - 0.5 };
  assert.ok(M.inspectPose(rotated, p, M.course(p)).edge < 0);
});

test('analytic curve derivatives, steering and straight entry/exit agree with finite differences', () => {
  const c = M.course(p), q = { bias: 0.56, amplitude: 1.1, phase: -0.2, shape: -0.04, first: 1 };
  for (const x of [c.start, c.end]) {
    const a = M.pathAt(x, q, p, c); near(a.y, q.bias); near(a.theta, 0); near(a.steer, 0);
  }
  for (const x of [3, 6, 11, 16, 23, 30]) {
    const h = 1e-4, a = M.pathAt(x, q, p, c), before = M.pathAt(x - h, q, p, c), after = M.pathAt(x + h, q, p, c);
    const dy = (after.y - before.y) / (2 * h);
    const ddy = (after.y - 2 * a.y + before.y) / (h * h);
    near(Math.tan(a.theta), dy, 1e-6);
    near(a.curvature, ddy / (1 + dy * dy) ** 1.5, 1e-6);
    near(a.handleRate, (after.handle - before.handle) / (2 * h) * (p.speed / 3.6) / a.norm, 1e-6);
  }
});

test('global displacement derivative bound covers every component including articulated parts', () => {
  const c = M.course(p), q = { bias: 0.56, amplitude: 1.3, phase: 0.2, shape: -0.12, first: -1 };
  const B = M.derivativeBounds(q, p);
  for (let x = c.start; x < c.end - 0.01; x += 0.07) {
    const h = 1e-5, a = M.pathAt(x, q, p, c), b = M.pathAt(x + h, q, p, c);
    const ga = M.geometry(a, p), gb = M.geometry(b, p);
    for (let j = 0; j < ga.length; j++) {
      const va = M.corners(ga[j]), vb = M.corners(gb[j]);
      for (let k = 0; k < va.length; k++) assert.ok(Math.hypot(va[k].x - vb[k].x, va[k].y - vb[k].y) / h <= B.motion);
    }
    assert.ok(Math.abs(a.curvature - b.curvature) / h <= B.curvatureDx);
    const knots = [c.start, c.poles[0].x - 0.35 * p.spacing, c.poles.at(-1).x + 0.35 * p.spacing, c.end];
    // Rate is piecewise smooth: a finite jump at a C2 join has no derivative bound.
    if (!knots.some(knot => knot >= x - 1e-9 && knot <= x + h + 1e-9)) {
      assert.ok(Math.abs(a.handleRate - b.handleRate) / h <= B.rateDx);
    }
  }
  const extreme = M.validate({ ...p, wheelbase: 2.6, front: 0.1, rear: 0.25, bikeWidth: 0.25, track: 0.65,
    sideWidth: 0.35, sideFront: 0.6, sideRear: 0.1, handleWidth: 0.5, handleX: 0.3, tireRadius: 0.45, tireWidth: 0.25 });
  const extremeBounds = M.derivativeBounds(q, extreme), ec = M.course(extreme);
  for (let x = ec.start + 0.01; x < ec.end - 0.01; x += 0.13) {
    const h = 1e-5;
    const a = M.geometry(M.pathAt(x, q, extreme, ec), extreme)[3];
    const b = M.geometry(M.pathAt(x + h, q, extreme, ec), extreme)[3];
    const va = M.corners(a), vb = M.corners(b);
    for (let i = 0; i < va.length; i++) assert.ok(Math.hypot(va[i].x-vb[i].x, va[i].y-vb[i].y)/h < extremeBounds.motion);
  }
});

test('mirroring the whole vehicle and route preserves clearance; speed affects rate only', () => {
  const q = { bias: 0.56, amplitude: 1.1, phase: 0.1, shape: 0, first: 1 };
  const a = M.evaluate(q, p, { step: 0.03 });
  const mirrored = M.evaluate({ ...q, bias: -q.bias, first: -q.first }, { ...p, side: 1 }, { step: 0.03 });
  near(a.minPole, mirrored.minPole); near(a.minEdge, mirrored.minEdge); near(a.maxSlip, mirrored.maxSlip);
  const slow = M.evaluate(q, { ...p, speed: p.speed / 2 }, { step: 0.03 });
  near(a.minPole, slow.minPole); near(a.maxSteer, slow.maxSteer); near(a.maxRate / 2, slow.maxRate);
  near(a.maxSlip, slow.maxSlip); near(a.duration * 2, slow.duration);
  const noLead = M.evaluate(q, { ...p, sideLead: 0 }, { step: 0.03 }); near(noLead.maxSlip, 0);
});

test('default search finds an alternating route with continuous geometric and steering margins', async () => {
  const r = await M.optimize(p, { yieldNow: () => Promise.resolve() });
  assert.ok(r.feasible); assert.ok(r.certified); assert.ok(r.clearance > p.margin);
  assert.ok(r.certifiedSteer <= p.maxSteer); assert.ok(r.certifiedRate <= p.maxRate);
  assert.ok(r.guard <= 0.005); assert.ok(r.sampleCount > r.samples.length);
  assert.equal(r.crossings.length, p.poleCount);
  for (let i = 0; i < r.crossings.length; i++) {
    const row = r.crossings[i]; assert.ok(row.front && row.rear && row.sideWheel);
    assert.ok(row.clearance > p.margin);
    if (i) assert.notEqual(row.side, r.crossings[i - 1].side);
  }
  // Independently recompute a still denser grid and compare to conservative bounds.
  const denser = M.evaluate(r.q, p, { step: r.dx / 2 });
  assert.ok(denser.sampledMinPole >= r.minPole);
  assert.ok(denser.sampledMinEdge >= r.minEdge);
  assert.ok(denser.maxSteer <= r.certifiedSteer); assert.ok(denser.maxRate <= r.certifiedRate);
  near(M.poseAtDistance(r, 0).x, r.c.start);
  near(M.poseAtDistance(r, r.length).x, r.c.end);
});

test('continuous sweep rejects a pole hit hidden between sparse poses', () => {
  const q = { bias: 0.5, amplitude: 0, phase: 0, shape: 0, first: 1 };
  const sparse = M.evaluate(q, p, { step: 100 });
  const checked = M.evaluate(q, p, { certify: true });
  assert.ok(sparse.sampledMinPole > 0);
  assert.ok(checked.minPole < 0);
  assert.equal(checked.feasible, false);
});

test('side-wheel forward motion retains its sign and endpoint rate limits are covered', () => {
  const asymmetric = M.validate({ ...p, wheelbase: 0.8, handleX: 0.5, track: 1.8, laneWidth: 8, spacing: 9, approach: 5 });
  const q = { bias: 0.9, amplitude: 0.3, shape: -0.15, phase: -3, first: 1 };
  const r = M.evaluate(q, asymmetric, { step: 0.003 });
  assert.ok(r.sideForward > 0, 'Outer-wheel curvature must not falsely count as inner-wheel reversal');
  const c = M.course(p), route = { bias: 0.56, amplitude: 1.2, phase: 0, shape: 0, first: 1 };
  const certified = M.evaluate(route, p, { certify: true });
  for (const x of [c.start + 1e-10, c.end - 1e-10, c.poles[0].x - 0.35 * p.spacing - 1e-10, c.poles[0].x - 0.35 * p.spacing + 1e-10]) {
    assert.ok(Math.abs(M.pathAt(x, route, p, c).handleRate) / M.RAD <= certified.certifiedRate);
  }
});

test('infeasible width, skipped poles and invalid measurements never receive a pass', async () => {
  const tight = { ...p, laneWidth: 2.1, spacing: 2.5, first: 1 };
  const r = await M.optimize(tight, { yieldNow: () => Promise.resolve() }); assert.equal(r.feasible, false);
  const skipped = M.evaluate({ bias: 1.7, amplitude: 0.3, phase: 0, shape: 0, first: 1 }, p);
  assert.ok(skipped.gateSlack < 0); assert.equal(skipped.feasible, false);
  assert.throws(() => M.validate({ spacing: NaN }));
  assert.throws(() => M.validate({ poleCount: 2.5 }));
  assert.throws(() => M.validate({ approach: 4, spacing: 9 }));
  assert.throws(() => M.validate({ track: 0.65, bikeWidth: 0.9, sideWidth: 1 }));
});
