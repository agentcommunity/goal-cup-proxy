// The run lifecycle (docs/UPLOAD-API.md): one main run that a later start replaces, practice runs,
// the start window, `continue` and its segments, and the close 24 hours after a run ends. All of
// it depends on time, so the Worker runs inside this process on a clock the test moves
// (test/helpers/inproc.ts). The same rules over real HTTP and real chunk bodies are in
// segments.test.ts.

import assert from "node:assert/strict";
import { describe, test } from "node:test";
import { Signer } from "../src/sign.ts";
import type { StartReceipt } from "../src/sign.ts";
import { GENESIS, makeChunk, openBody, sha256, verifyReceipt } from "./helpers/client.ts";
import { type Opened, T0, WINDOW, lifecycle, rig } from "./helpers/rig.ts";
import { TEST_SEED } from "./helpers/worker.ts";

// The chunk upload needs the Workers runtime's FixedLengthStream; a pass-through stands in for it
// here (test/quota.test.ts does the same; test/integration.test.ts has the real one).
const g = globalThis as unknown as { FixedLengthStream?: unknown };
g.FixedLengthStream ??= class extends TransformStream<Uint8Array, Uint8Array> {
  constructor(_length: number) {
    super();
  }
};

const publicKey = (await Signer.fromSeedHex(TEST_SEED)).publicKeyHex;
/** The default run length (RUN_HOURS 168), in seconds. */
const WEEK = 168 * 3600;

function assertError(r: { status: number; body: { error?: string; message?: string } }, status: number, code: string) {
  assert.equal(r.status, status, JSON.stringify(r.body));
  assert.equal(r.body.error, code);
  assert.equal(typeof r.body.message, "string");
  assert.deepEqual(Object.keys(r.body).sort(), ["error", "message"]);
}


describe("one main run per account", () => {
  lifecycle("a second start in the window replaces the first, whose heartbeat says stop/replaced", async (r) => {
    const [c1, c2, c3] = [await r.cred("alice"), await r.cred("alice"), await r.cred("alice")];
    const bob = await r.cred("bob");
    const bobRun = await r.start(bob, "bob-1");
    const a = await r.start(c1, "s1");
    r.after(60);
    const b = await r.start(c2, "s2"); // was 409 "already has an open run"
    assert.notEqual(b.run_id, a.run_id);

    // The first run is told to stop, and why; its deadline is unchanged.
    const hb = await r.beat(c1, a.run_id, a.entry_id);
    assert.equal(hb.status, 200, JSON.stringify(hb.body));
    // seconds_left counts to deadline_at, which a replacement does not move; stop is what ends it.
    assert.deepEqual(hb.body, { stop: true, deadline_at: a.deadline_at, seconds_left: WEEK - 60, reason: "replaced", sealed: false });
    const pos = await r.get(c1, a.run_id);
    assert.equal(pos.body.stop, true);
    assert.equal(pos.body.status, "open");
    assert.equal(pos.body.ends_at, "2026-11-06T09:01:00Z");
    assert.equal(pos.body.deadline_at, a.deadline_at);

    // The second runs on; another account's run is not touched; the replaced run keeps its credential.
    assert.deepEqual((await r.beat(c2, b.run_id, b.entry_id)).body, { stop: false, deadline_at: b.deadline_at, seconds_left: WEEK, reason: null, sealed: false });
    assert.deepEqual((await r.beat(bob, bobRun.run_id, bobRun.entry_id)).body, { stop: false, deadline_at: bobRun.deadline_at, seconds_left: WEEK - 60, reason: null, sealed: false });
    assert.equal((await r.get(c2, b.run_id)).body.ends_at, b.deadline_at);

    // A lost reply is still the same run, not a third start.
    const again = await r.open(c2, "s2");
    assert.equal(again.status, 200, JSON.stringify(again.body));
    assert.deepEqual(again.body, b);
    assert.equal((await r.beat(c2, b.run_id, b.entry_id)).body.stop, false);

    // A third start replaces the second; the first keeps the instant it was replaced.
    r.after(60);
    const c = await r.start(c3, "s3");
    assert.equal((await r.get(c1, a.run_id)).body.ends_at, "2026-11-06T09:01:00Z");
    assert.deepEqual((await r.beat(c2, b.run_id, b.entry_id)).body, { stop: true, deadline_at: b.deadline_at, seconds_left: WEEK - 60, reason: "replaced", sealed: false });
    assert.equal((await r.beat(c3, c.run_id, c.entry_id)).body.stop, false);
  });

  lifecycle("a replaced run takes its final heartbeat for 24 hours, and nothing after", async (r) => {
    const [c1, c2] = [await r.cred("alice"), await r.cred("alice")];
    const a = await r.start(c1, "s1");
    const head = await r.hold("alice", a.run_id, a.entry_id);
    r.after(60);
    await r.start(c2, "s2");

    // Replaced, so over: the final heartbeat commits the end the server holds, which seals it.
    const f = await r.finish(c1, a.run_id, a.entry_id, head);
    assert.equal(f.stop, true);
    assert.equal(f.sealed, true);
    assert.equal((await r.get(c1, a.run_id)).body.status, "sealed");

    // 24 hours after it was replaced it takes nothing.
    r.after(24 * 3600 + 1);
    assertError(await r.beat(c1, a.run_id, a.entry_id), 410, "closed");
    assert.equal((await r.get(c1, a.run_id)).body.status, "sealed");
  });
});

describe("the start window and practice runs", () => {
  lifecycle("after the window a start is 403 and continue works", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    r.at("2026-11-06T10:00:01Z");
    const bob = await r.cred("bob");
    const late = await r.open(bob, "bob-1");
    assertError(late, 403, "forbidden");
    assert.equal(late.body.message, "after the start window you can only `goalcup continue` your run");
    const c = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    assert.equal(c.body.segment, 1);
    // The window is the edges in: the last second is inside.
    r.at("2026-11-06T10:00:00Z");
    assert.equal((await r.open(await r.cred("carol"), "carol-1")).status, 201);
  });

  lifecycle("before the window opens a start is 403, and a start with no window configured is always open", async (r) => {
    const dave = await r.cred("dave");
    assertError(await r.open(dave, "d1"), 403, "forbidden");
    r.at("2026-11-06T08:00:00Z");
    assert.equal((await r.open(dave, "d1")).status, 201);
  }, WINDOW, "2026-11-06T07:59:59Z");

  lifecycle("no window configured means always open", async (r) => {
    assert.equal((await r.open(await r.cred("erin"), "e1")).status, 201);
    r.at("2031-01-01T00:00:00Z");
    assert.equal((await r.open(await r.cred("frank"), "f1")).status, 201);
  }, {});

  lifecycle("practice never replaces and ends after PRACTICE_RUN_MINUTES", async (r) => {
    const [comp, p1c, p2c] = [await r.cred("dave"), await r.cred("dave", "practice"), await r.cred("dave", "practice")];
    const c = await r.start(comp, "comp");
    const p1 = await r.start(p1c, "pr1");
    assert.equal(Date.parse(p1.deadline_at) - Date.parse(p1.official_start), 10 * 60 * 1000);
    for (const [token, run] of [[p1c, p1], [comp, c]] as const) {
      const chunk = makeChunk(run.entry_id, 0, 3, GENESIS, run.entry_id);
      const put = await r.api.call("PUT", `/v1/runs/${run.run_id}/chunks/0`, { token, body: chunk.body, headers: { ...chunk.headers, "content-length": String(chunk.body.length) } });
      assert.equal(put.status, 200, JSON.stringify(put.body));
    }
    r.after(60);
    const p2 = await r.start(p2c, "pr2"); // a second practice run does not stop the first
    for (const [token, run] of [[comp, c], [p1c, p1], [p2c, p2]] as const) {
      assert.equal((await r.beat(token, run.run_id, run.entry_id)).body.stop, false);
    }

    // The first practice run ends at 09:10, the second at 09:11; the competition run goes on.
    r.at("2026-11-06T09:10:00Z");
    assert.deepEqual((await r.beat(p1c, p1.run_id, p1.entry_id)).body, { stop: true, deadline_at: p1.deadline_at, seconds_left: 0, reason: "deadline", sealed: false });
    assert.equal((await r.beat(p2c, p2.run_id, p2.entry_id)).body.stop, false);
    assert.equal((await r.beat(comp, c.run_id, c.entry_id)).body.stop, false);

    // A competition start replaces the competition run only, and practice is outside the window.
    const next = await r.start(await r.cred("dave"), "comp2");
    assert.equal((await r.beat(comp, c.run_id, c.entry_id)).body.reason, "replaced");
    assert.equal((await r.beat(p2c, p2.run_id, p2.entry_id)).body.stop, false);
    r.at("2026-11-06T12:00:00Z");
    assert.equal((await r.open(await r.cred("dave", "practice"), "pr3")).status, 201);
    assert.equal((await r.get(await r.cred("dave"), next.run_id)).status, 404); // a new credential sees no run before it opens one

    // Practice runs are stored under the practice event, competition runs under the event.
    const keys = [...r.bucket.map.keys()].filter((k) => k.startsWith("traces/"));
    assert.ok(keys.some((k) => k.startsWith(`traces/practice/${p1.run_id}/${p1.entry_id}/`)), keys.join("\n"));
    assert.ok(keys.some((k) => k.startsWith(`traces/cup/${c.run_id}/${c.entry_id}/`)), keys.join("\n"));
  });

  lifecycle("PRACTICE_RUN_MINUTES sets the practice length", async (r) => {
    const p = await r.start(await r.cred("gina", "practice"), "g1");
    assert.equal(Date.parse(p.deadline_at) - Date.parse(p.official_start), 90 * 1000);
  }, { PRACTICE_RUN_MINUTES: "1.5" });
});

describe("continue and segments", () => {
  lifecycle("continue mints segment 1 with its own entry and receipt", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const first = await r.api.call("POST", `/v1/runs/${a.run_id}/receipt`, { token: alice, json: { entry_id: a.entry_id, chain_head: "b".repeat(64) } });
    assert.equal(first.status, 200, JSON.stringify(first.body));
    r.after(300);

    const c = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    assert.deepEqual(Object.keys(c.body), ["entry_id", "segment", "official_start", "deadline_at", "seconds_left"]);
    assert.equal(c.body.seconds_left, WEEK - 300); // what is left of the run, not a new run's length
    assert.match(c.body.entry_id, /^e[0-9a-f]{16}$/);
    assert.notEqual(c.body.entry_id, a.entry_id);
    assert.equal(c.body.segment, 1);
    assert.equal(c.body.official_start, a.official_start); // the run's clock, not a new one
    assert.equal(c.body.deadline_at, a.deadline_at);

    // Its receipt claims this entry's own connector facts and session, the run's start and deadline.
    const r1 = await r.api.call("POST", `/v1/runs/${a.run_id}/receipt`, { token: alice, json: { entry_id: c.body.entry_id, chain_head: "d".repeat(64) } });
    assert.equal(r1.status, 200, JSON.stringify(r1.body));
    const receipt: StartReceipt = r1.body.start_receipt;
    assert.deepEqual(receipt.claims, {
      entry_id: c.body.entry_id, connector_id: "other-cli", workspace: "/w/two", session_id: "s-two",
      started_at: a.official_start, deadline_at: a.deadline_at, manifest_sha256: "1".repeat(64),
      chain_head: "d".repeat(64), connector_dir_sha256: "1".repeat(64),
    });
    assert.equal(await verifyReceipt(receipt, publicKey), true);
    // Segment 0 keeps its own.
    const again = await r.api.call("POST", `/v1/runs/${a.run_id}/receipt`, { token: alice, json: { entry_id: a.entry_id, chain_head: "b".repeat(64) } });
    assert.deepEqual(again.body.start_receipt, first.body.start_receipt);
    assert.equal(again.body.start_receipt.claims.workspace, openBody().workspace);

    // The position answers the current entry, or the one asked for.
    const now = await r.get(alice, a.run_id);
    assert.equal(now.body.entry_id, c.body.entry_id);
    assert.equal(now.body.segment, 1);
    assert.equal(now.body.offset, 0);
    const old = await r.get(alice, a.run_id, a.entry_id);
    assert.equal(old.body.entry_id, a.entry_id);
    assert.equal(old.body.segment, 0);
    assert.deepEqual(Object.keys(old.body).sort(), [
      "deadline_at", "ends_at", "entry_id", "final", "head", "last_ack", "next_seq", "official_start", "offset", "run_id", "sealed", "segment", "status", "stop",
    ]);
    assertError(await r.get(alice, a.run_id, "e" + "7".repeat(16)), 404, "not_found");
    assertError(await r.get(alice, a.run_id, "nope"), 400, "malformed");

    // A repeat of the same continue is the same answer, not a third segment.
    const repeat = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(repeat.status, 200, JSON.stringify(repeat.body));
    assert.deepEqual(repeat.body, c.body);
    assert.equal((await r.get(alice, a.run_id)).body.segment, 1);
  });

  lifecycle("a lost-reply repeat of the open is the same run, also after continue", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    assert.equal((await r.cont(alice, a.run_id, a.entry_id, "s-two")).status, 201);
    const again = await r.open(alice, "s1");
    assert.equal(again.status, 200, JSON.stringify(again.body));
    assert.deepEqual(again.body, a); // segment 0's entry, the run's own start and deadline
    assertError(await r.open(alice, "s-two"), 409, "conflict"); // continue's session is not the open call's
  });

  lifecycle("stale previous_entry_id is 409", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const c1 = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(c1.status, 201);
    // The latest entry is c1's; the first one is no longer a place to continue from.
    assertError(await r.cont(alice, a.run_id, a.entry_id, "s-three"), 409, "conflict");
    assertError(await r.cont(alice, a.run_id, "e" + "5".repeat(16), "s-three"), 409, "conflict");
    assertError(await r.cont(alice, a.run_id, "not-an-entry", "s-three"), 400, "malformed");
    assertError(await r.cont(alice, a.run_id, c1.body.entry_id, ""), 400, "malformed");
    assert.equal((await r.get(alice, a.run_id)).body.segment, 1);
    const c2 = await r.cont(alice, a.run_id, c1.body.entry_id, "s-three");
    assert.equal(c2.status, 201, JSON.stringify(c2.body));
    assert.equal(c2.body.segment, 2);
    // Another credential, another run, a foreign run id: not found.
    assertError(await r.cont(await r.cred("bob"), a.run_id, c2.body.entry_id, "s-four"), 404, "not_found");
    assertError(await r.api.call("GET", `/v1/runs/${a.run_id}/continue`, { token: alice }), 405, "method_not_allowed");
  });

  lifecycle("continue after ends_at is 410", async (r) => {
    const [c1, c2] = [await r.cred("alice"), await r.cred("alice")];
    const a = await r.start(c1, "s1");
    r.after(60);
    await r.start(c2, "s2"); // replaced: its end is now
    assertError(await r.cont(c1, a.run_id, a.entry_id, "s-two"), 410, "closed");

    const [d, e] = [await r.cred("dave"), await r.cred("erin")];
    const run = await r.start(d, "d1");
    r.at(run.deadline_at);
    assertError(await r.cont(d, run.run_id, run.entry_id, "s-two"), 410, "closed");
    // One second before the deadline it still worked (a run of 168 hours has this second).
    r.at("2026-11-06T09:00:00Z");
    const fresh = await r.start(e, "e1");
    r.at(new Date(Date.parse(fresh.deadline_at) - 1000).toISOString());
    assert.equal((await r.cont(e, fresh.run_id, fresh.entry_id, "s-two")).status, 201);
  });

  lifecycle("an older segment commits its end and is sealed by its entry_id", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const h0 = await r.hold("alice", a.run_id, a.entry_id);
    const c = (await r.cont(alice, a.run_id, a.entry_id, "s-two")).body;
    const h1 = await r.hold("alice", a.run_id, c.entry_id);

    // Each entry commits its own end, whenever it likes, and the position of each is its own.
    assert.equal((await r.finish(alice, a.run_id, a.entry_id, h0)).sealed, true);
    const old = (await r.get(alice, a.run_id, a.entry_id)).body;
    assert.deepEqual([old.final, old.sealed], [{ seq: 4, head: h0 }, true]);
    const now = (await r.get(alice, a.run_id)).body;
    assert.deepEqual([now.final, now.head, now.sealed], [null, h1, false]);
    assert.equal((await r.get(alice, a.run_id)).body.status, "open");
    assertError(await r.beat(alice, a.run_id, "e" + "6".repeat(16)), 404, "not_found");
    // A repeat of the older final heartbeat is the same answer.
    assert.equal((await r.finish(alice, a.run_id, a.entry_id, h0)).sealed, true);
  });

  lifecycle("a segment sealed before the end leaves the run open", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const h0 = await r.hold("alice", a.run_id, a.entry_id);
    assert.equal((await r.finish(alice, a.run_id, a.entry_id, h0)).sealed, true);
    assert.equal((await r.get(alice, a.run_id)).body.status, "open");
    assert.equal((await r.get(alice, a.run_id)).body.stop, false);

    // So the run can go on: another segment, with a heartbeat that says go on.
    const c = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    assert.equal((await r.beat(alice, a.run_id, c.body.entry_id)).body.stop, false);
    assert.equal((await r.get(alice, a.run_id)).body.status, "open");

    // The entry that was sealed takes no more chain; the run is not its alone.
    const dead = await r.api.call("POST", `/v1/runs/${a.run_id}/receipt`, { token: alice, json: { entry_id: a.entry_id, chain_head: "b".repeat(64) } });
    assertError(dead, 410, "closed");
  });

  lifecycle("a run whose current entry is sealed reads as sealed once the run is over", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const head = await r.hold("alice", a.run_id, a.entry_id);
    assert.equal((await r.finish(alice, a.run_id, a.entry_id, head)).sealed, true);
    assert.equal((await r.get(alice, a.run_id)).body.status, "open");
    r.at(a.deadline_at);
    assert.equal((await r.get(alice, a.run_id)).body.status, "sealed");
    assertError(await r.cont(alice, a.run_id, a.entry_id, "s-two"), 410, "closed");
  });
});

describe("the time left, on the server's clock (WP11)", () => {
  lifecycle("open, continue and every heartbeat say seconds_left, down to 0 at the deadline and after it", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    // The open answer: the whole run, counted on the server's clock.
    assert.equal((a as Opened & { seconds_left: number }).seconds_left, WEEK);
    r.after(3600);
    assert.equal((await r.beat(alice, a.run_id, a.entry_id)).body.seconds_left, WEEK - 3600);
    // A continue says what is left of the run, not a new run's length.
    const c = await r.cont(alice, a.run_id, a.entry_id, "s-two");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    assert.equal(c.body.seconds_left, WEEK - 3600);
    // A lost-reply repeat of the open answers the time left now.
    r.after(60);
    assert.equal((await r.open(alice, "s1")).body.seconds_left, WEEK - 3660);

    // One second before the deadline, one second is left and the run goes on.
    r.at(new Date(Date.parse(a.deadline_at) - 1000).toISOString());
    assert.deepEqual((await r.beat(alice, a.run_id, c.body.entry_id)).body, { stop: false, deadline_at: a.deadline_at, seconds_left: 1, reason: null, sealed: false });
    // At the deadline nothing is left, and the run stops; later it stays 0, never negative.
    r.at(a.deadline_at);
    assert.deepEqual((await r.beat(alice, a.run_id, c.body.entry_id)).body, { stop: true, deadline_at: a.deadline_at, seconds_left: 0, reason: "deadline", sealed: false });
    r.after(3600);
    assert.deepEqual((await r.beat(alice, a.run_id, c.body.entry_id)).body, { stop: true, deadline_at: a.deadline_at, seconds_left: 0, reason: "deadline", sealed: false });
  });
});

describe("the close 24 hours after a run ends", () => {
  lifecycle("at +24 h a run whose current entry is sealed is sealed, else incomplete", async (r) => {
    const [alice, bob] = [await r.cred("alice"), await r.cred("bob")];
    const x = await r.start(alice, "x1");
    const y = await r.start(bob, "y1");
    const hx = await r.hold("alice", x.run_id, x.entry_id);
    await r.hold("bob", y.run_id, y.entry_id);
    assert.equal((await r.finish(alice, x.run_id, x.entry_id, hx)).sealed, true);

    // Both accounts start again at 09:10, which ends these two runs then.
    r.at("2026-11-06T09:10:00Z");
    await r.start(await r.cred("alice"), "x2");
    await r.start(await r.cred("bob"), "y2");
    // The sealed one reads as sealed at once; the other waits for its wrap-up.
    assert.equal((await r.get(alice, x.run_id)).body.status, "sealed");
    assert.equal((await r.get(bob, y.run_id)).body.status, "open");
    // bob commits an end the server never gets the chain up to: not sealed.
    const ahead = await r.beat(bob, y.run_id, y.entry_id, { seq: 9, head: "9".repeat(64), final: true });
    assert.equal(ahead.status, 200, JSON.stringify(ahead.body));
    assert.equal(ahead.body.sealed, false);

    // Exactly 24 hours later is still inside; one second more is not.
    r.at("2026-11-07T09:10:00Z");
    assert.equal((await r.get(bob, y.run_id)).body.status, "open");
    assert.equal((await r.beat(bob, y.run_id, y.entry_id)).status, 200);
    r.after(1);
    assert.equal((await r.get(bob, y.run_id)).body.status, "incomplete");
    assert.equal((await r.get(alice, x.run_id)).body.status, "sealed");
    assert.equal(r.storedRun(y.run_id).status, "incomplete"); // written, not only computed
    assert.equal(r.storedRun(x.run_id).status, "sealed");

    // Nothing is accepted after that; the position still answers.
    assertError(await r.beat(bob, y.run_id, y.entry_id), 410, "closed");
    assertError(await r.beat(alice, x.run_id, x.entry_id), 410, "closed");
    assertError(await r.cont(bob, y.run_id, y.entry_id, "s-two"), 410, "closed");
    const gone = (await r.get(alice, x.run_id)).body;
    assert.deepEqual([gone.status, gone.sealed], ["sealed", true]);
  });

  lifecycle("an older segment can still finish until the run closes", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    const h0 = await r.hold("alice", a.run_id, a.entry_id);
    const c = (await r.cont(alice, a.run_id, a.entry_id, "s-two")).body;
    const h1 = await r.hold("alice", a.run_id, c.entry_id);
    assert.equal((await r.finish(alice, a.run_id, c.entry_id, h1)).sealed, true);
    r.at(a.deadline_at);
    assert.equal((await r.get(alice, a.run_id)).body.status, "sealed");

    // The run reads sealed, but segment 0 has not sent its end yet: it may, for 24 hours.
    r.after(3600);
    assert.equal((await r.get(alice, a.run_id, a.entry_id)).body.sealed, false);
    assert.equal((await r.finish(alice, a.run_id, a.entry_id, h0)).sealed, true);
    assert.equal((await r.get(alice, a.run_id, a.entry_id)).body.sealed, true);
    assert.equal((await r.get(alice, a.run_id)).body.status, "sealed");
  });
});

describe("what a seal needs", () => {
  lifecycle("no final heartbeat, or a chain behind the committed end, is not sealed", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    // The chain held, but no end committed: an ordinary heartbeat seals nothing.
    const head = await r.hold("alice", a.run_id, a.entry_id);
    assert.equal((await r.beat(alice, a.run_id, a.entry_id, { seq: 4, head })).body.sealed, false);
    assert.equal((await r.get(alice, a.run_id)).body.sealed, false);

    // An end the server does not hold the chain up to yet.
    const f = await r.beat(alice, a.run_id, a.entry_id, { seq: 9, head: "9".repeat(64), final: true });
    assert.equal(f.status, 200, JSON.stringify(f.body));
    assert.equal(f.body.sealed, false);
    r.at(a.deadline_at);
    const again = await r.beat(alice, a.run_id, a.entry_id, { seq: 9, head: "9".repeat(64), final: true });
    assert.equal(again.body.sealed, false); // past the end too: an end is committed, the chain is behind it
    assert.equal((await r.get(alice, a.run_id)).body.sealed, false);
  }, {});
});

describe("facts about the machine and the connection", () => {
  const machine = { os: "macOS", os_version: "26.0", cpus: 8, memory_gb: 32.5, ac_power: true, wsl2: false, sandbox: null };

  test("machine, client IP and country are stored on the run, the latest pair moves on continue, and never echoed", async () => {
    const r = rig({}, T0, { country: "DE" });
    try {
      const alice = await r.cred("alice");
      const opened = await r.open(alice, "s1", { machine }, { "cf-connecting-ip": "203.0.113.9" });
      assert.equal(opened.status, 201, JSON.stringify(opened.body));
      const run = opened.body as Opened;
      assert.deepEqual(r.storedRun(run.run_id).machine, machine);
      assert.equal(r.storedRun(run.run_id).client_ip, "203.0.113.9");
      assert.equal(r.storedRun(run.run_id).client_country, "DE");

      const c = await r.cont(alice, run.run_id, run.entry_id, "s-two", {}, { "cf-connecting-ip": "198.51.100.7" });
      assert.equal(c.status, 201, JSON.stringify(c.body));
      assert.equal(r.storedRun(run.run_id).last_client_ip, "198.51.100.7");
      assert.equal(r.storedRun(run.run_id).client_ip, "203.0.113.9"); // the open call's pair stays
      assert.deepEqual(r.storedRun(run.run_id).machine, machine); // the open call's facts stay

      // Not in any answer.
      const answers = [
        opened.body,
        c.body,
        (await r.get(alice, run.run_id)).body,
        (await r.beat(alice, run.run_id, c.body.entry_id)).body,
        (await r.open(alice, "s1")).body,
      ];
      for (const text of answers.map((a) => JSON.stringify(a))) {
        assert.ok(!/203\.0\.113\.9|198\.51\.100\.7|"DE"|macOS|machine|client_/.test(text), text);
      }
    } finally {
      r.stop();
    }
  });

  test("string caps count UTF-16 code units: 32 emoji fill a 64-character field", async () => {
    const r = rig({}, T0);
    try {
      const emoji = "\u{1F600}";
      assert.equal((await r.open(await r.cred("alice"), "s1", { harness_version: emoji.repeat(32) })).status, 201);
      assertError(await r.open(await r.cred("bob"), "s2", { harness_version: emoji.repeat(33) }), 400, "malformed");
      assert.equal((await r.open(await r.cred("carol"), "s3", { connector_id: "c".repeat(128) })).status, 201);
      assertError(await r.open(await r.cred("dave"), "s4", { connector_id: "c".repeat(129) }), 400, "malformed");
    } finally {
      r.stop();
    }
  });

  test("open and continue take proxy_version or tracer_version, store it as tracer_version, and refuse two that differ", async () => {
    const r = rig({}, T0);
    try {
      const versions = (run: string) => (r.storedRun(run).entries as { tracer_version: string }[]).map((e) => e.tracer_version);
      const alice = await r.cred("alice");
      const opened = await r.open(alice, "s1", { proxy_version: "0.7.0" });
      assert.equal(opened.status, 201, JSON.stringify(opened.body));
      const run = opened.body as Opened;
      assert.deepEqual(versions(run.run_id), ["0.7.0"]);
      // A continue from an older release says tracer_version; one from this release may say both.
      const c1 = await r.cont(alice, run.run_id, run.entry_id, "s-two", { tracer_version: "0.6.0" });
      assert.equal(c1.status, 201, JSON.stringify(c1.body));
      assertError(await r.cont(alice, run.run_id, c1.body.entry_id, "s-three", { proxy_version: "0.7.1", tracer_version: "0.7.0" }), 400, "malformed");
      assertError(await r.cont(alice, run.run_id, c1.body.entry_id, "s-three", { tracer_version: undefined }), 400, "malformed");
      assertError(await r.cont(alice, run.run_id, c1.body.entry_id, "s-three", { tracer_version: undefined, proxy_version: "x".repeat(65) }), 400, "malformed");
      const c2 = await r.cont(alice, run.run_id, c1.body.entry_id, "s-three", { proxy_version: "0.7.1", tracer_version: "0.7.1" });
      assert.equal(c2.status, 201, JSON.stringify(c2.body));
      assert.deepEqual(versions(run.run_id), ["0.7.0", "0.6.0", "0.7.1"]);
      assertError(await r.open(await r.cred("bob"), "s4", { proxy_version: "0.7.0", tracer_version: "0.6.0" }), 400, "malformed");
      assertError(await r.open(await r.cred("carol"), "s5", { proxy_version: undefined }), 400, "malformed");
    } finally {
      r.stop();
    }
  });

  test("a run opened with no machine, no IP and no country stores nulls", async () => {
    const r = rig({}, T0);
    try {
      const run = await r.start(await r.cred("alice"), "s1");
      assert.equal(r.storedRun(run.run_id).machine, null);
      assert.equal(r.storedRun(run.run_id).client_ip, null);
      assert.equal(r.storedRun(run.run_id).client_country, null);
      assert.equal(r.storedRun(run.run_id).last_client_ip ?? null, null);
      assert.equal((await r.open(await r.cred("bob"), "b1", { machine: null })).status, 201); // null is absent
    } finally {
      r.stop();
    }
  });

  test("machine must be a flat object of at most 4 KiB", async () => {
    const r = rig({}, T0);
    try {
      const exactly = (n: number) => ({ k: "x".repeat(n - '{"k":""}'.length) });
      assert.equal(JSON.stringify(exactly(4096)).length, 4096);
      assert.equal((await r.open(await r.cred("alice"), "s1", { machine: exactly(4096) })).status, 201);
      for (const bad of [exactly(4097), { nested: { a: 1 } }, { list: [1] }, [1, 2], "linux", 5, true]) {
        assertError(await r.open(await r.cred("bob"), "b1", { machine: bad }), 400, "malformed");
      }
    } finally {
      r.stop();
    }
  });
});

describe("--dev runs and the minimum goalcup version (D70)", () => {
  const DEV_COMPETES = "A --dev run never enters the competition. Start without --dev.";

  lifecycle("a dev competition open is refused, for everyone", async (r) => {
    for (const member of ["alice", "root"]) {
      const res = await r.open(await r.cred(member), `s-${member}`, { dev: true });
      assertError(res, 403, "forbidden");
      assert.equal(res.body.message, DEV_COMPETES);
    }
  }, WINDOW);

  lifecycle("a dev practice open from any member opens", async (r) => {
    for (const member of ["alice", "bob", "root"]) {
      const res = await r.open(await r.cred(member, "practice"), `s-${member}`, { dev: true });
      assert.equal(res.status, 201, JSON.stringify(res.body));
    }
  }, WINDOW);

  lifecycle("a dev continue of a competition run is refused", async (r) => {
    const root = await r.cred("root");
    const a = await r.start(root, "s1");
    const res = await r.cont(root, a.run_id, a.entry_id, "s-two", { dev: true });
    assertError(res, 403, "forbidden");
    assert.equal(res.body.message, DEV_COMPETES);
  }, WINDOW);

  lifecycle("a dev continue of a practice run is open to any member, and of an unknown run is 404", async (r) => {
    const alice = await r.cred("alice", "practice");
    const a = await r.start(alice, "s1");
    assert.equal((await r.cont(alice, a.run_id, a.entry_id, "s-two", { dev: true })).status, 201);
    assertError(await r.cont(alice, "r0000000000000000", a.entry_id, "s-x", { dev: true }), 404, "not_found");
  }, WINDOW);

  lifecycle("an open or continue without dev is unchanged", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    assert.equal((await r.open(await r.cred("bob", "practice"), "s2", { dev: false })).status, 201);
    assert.equal((await r.cont(alice, a.run_id, a.entry_id, "x", { dev: false })).status, 201);
  }, WINDOW);

  lifecycle("dev must be a boolean", async (r) => {
    for (const bad of ["yes", 1, 0, "true", null, [], {}]) {
      assertError(await r.open(await r.cred("root", "practice"), "s1", { dev: bad }), 400, "malformed");
    }
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s2");
    assertError(await r.cont(alice, a.run_id, a.entry_id, "s-two", { dev: "yes" }), 400, "malformed");
  }, WINDOW);

  lifecycle("an old goalcup cannot open a competition run once a minimum is set", async (r) => {
    const [alice, bob, carol] = [await r.cred("alice"), await r.cred("bob"), await r.cred("carol")];
    r.env.MIN_COMPETITION_VERSION = "0.7.0";
    assertError(await r.open(alice, "s1", { proxy_version: "0.6.1" }), 403, "forbidden");
    assert.equal((await r.open(alice, "s1", { proxy_version: undefined, tracer_version: "0.6.9" })).status, 403); // the older name counts too
    assertError(await r.open(alice, "s1", { proxy_version: "unknown" }), 403, "forbidden"); // unknown while a minimum is set
    assert.equal((await r.open(alice, "s1", { proxy_version: "0.7.0" })).status, 201);
    assert.equal((await r.open(bob, "s2", { proxy_version: "1.0.0" })).status, 201);
    assert.equal((await r.open(carol, "s3", { proxy_version: "0.10.0" })).status, 201); // numbers, not text
  }, WINDOW);

  lifecycle("the version answer comes before the dev answers", async (r) => {
    const alice = await r.cred("alice");
    r.env.MIN_COMPETITION_VERSION = "0.7.0";
    const res = await r.open(alice, "s1", { proxy_version: "0.6.1", dev: true });
    assertError(res, 403, "forbidden");
    assert.notEqual(res.body.message, DEV_COMPETES);
  }, WINDOW);

  lifecycle("a version with a suffix compares by its numbers", async (r) => {
    const [alice, bob, carol] = [await r.cred("alice"), await r.cred("bob"), await r.cred("carol")];
    r.env.MIN_COMPETITION_VERSION = "0.7.0";
    assert.equal((await r.open(alice, "s1", { proxy_version: "0.7.0-dev" })).status, 201);
    assert.equal((await r.open(bob, "s2", { proxy_version: "goalcup 0.8.2+abc" })).status, 201);
    assertError(await r.open(carol, "s3", { proxy_version: "0.6.99-rc.1" }), 403, "forbidden");
  });

  lifecycle("a retried open of an existing run is answered even below the minimum", async (r) => {
    const alice = await r.cred("alice");
    const a = await r.start(alice, "s1");
    r.env.MIN_COMPETITION_VERSION = "9.0.0";
    const again = await r.open(alice, "s1", { proxy_version: "0.1.0" });
    assert.equal(again.status, 200, JSON.stringify(again.body));
    assert.equal(again.body.run_id, a.run_id);
    assertError(await r.open(alice, "other-session", { proxy_version: "0.1.0" }), 409, "conflict"); // not a lost reply: unchanged
  });

  lifecycle("a malformed minimum refuses opens in every mode and leaves heartbeats, get, chunk uploads and continue alone", async (r) => {
    const [alice, bob, carol] = [await r.cred("alice"), await r.cred("bob"), await r.cred("bob", "practice")];
    const a = await r.start(alice, "s1");
    r.env.MIN_COMPETITION_VERSION = "0.7";
    assertError(await r.open(bob, "s2"), 500, "misconfigured");
    assert.equal((await r.beat(alice, a.run_id, a.entry_id)).status, 200);
    assert.equal((await r.get(alice, a.run_id)).status, 200);
    const chunk = makeChunk(a.entry_id, 0, 3, GENESIS, "m");
    const put = await r.api.call("PUT", `/v1/runs/${a.run_id}/chunks/0`, { token: alice, body: chunk.body, headers: { ...chunk.headers, "content-length": String(chunk.body.length) } });
    assert.equal(put.status, 200, JSON.stringify(put.body)); // a running run keeps uploading
    assertError(await r.open(carol, "s3"), 500, "misconfigured"); // practice too
    assert.equal((await r.cont(alice, a.run_id, a.entry_id, "s-two")).status, 201);
  });
});
