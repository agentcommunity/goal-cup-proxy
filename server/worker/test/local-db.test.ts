// LocalDb on an in-memory bucket: the rules that need a clock the HTTP tests cannot move.

import assert from "node:assert/strict";
import { test } from "node:test";
import { boundCredentialExpiry, endsAt, isPastClose, isPastEnd, statusAt } from "../src/db.ts";
import type { ContinueRunArgs, HeartbeatArgs, OpenRunArgs, RunMode } from "../src/db.ts";
import { LocalDb } from "../src/db-local.ts";
import { sha256Hex } from "../src/util.ts";
import { FakeBucket, asR2 } from "./helpers/fake-r2.ts";

let devices = 0;
/** The sha256 of a fresh run credential for `member`, through the device flow. */
async function credential(db: LocalDb, member = "m1", mode: RunMode = "competition", at = "2026-11-06T09:00:00Z"): Promise<string> {
  const n = ++devices;
  const dev = {
    device_sha256: await sha256Hex(`device-${n}`),
    user_sha256: await sha256Hex(`USER${n}`),
    purpose: "run" as const,
    mode,
    connector_id: "codex-cli",
    tracer_version: "1",
    created_at: at,
    expires_at: "2099-01-01T00:00:00Z",
  };
  await db.createDevice(dev);
  assert.equal(await db.approveDevice(`USER${n}`, member, true, at), "ok");
  const got = await db.pollDevice(dev.device_sha256, at);
  assert.equal(got.kind, "approved");
  return sha256Hex(got.kind === "approved" ? (got.credential_plain ?? "") : "");
}

const id = (prefix: string, n: number) => `${prefix}${String(n).padStart(16, "0")}`;

const openArgs = (n: number, session: string, startedAt: string, deadlineAt: string, cred: string, over: Partial<OpenRunArgs> = {}): OpenRunArgs => ({
  member_id: "m1",
  event_id: "cup",
  mode: "competition",
  run_id: id("r", n),
  entry_id: id("e", n),
  official_start: startedAt,
  deadline_at: deadlineAt,
  connector_id: "codex-cli",
  manifest_sha256: "a".repeat(64),
  connector_dir_sha256: "c".repeat(64),
  workspace: "/w",
  session_id: session,
  harness_version: "1",
  tracer_version: "1",
  repo_url: "https://github.com/a/b",
  machine: null,
  client_ip: null,
  client_country: null,
  credential: cred,
  credential_expires_at: boundCredentialExpiry(deadlineAt),
  ...over,
});

const contArgs = (entryId: string, previous: string, session: string, at: string, over: Partial<ContinueRunArgs> = {}): ContinueRunArgs => ({
  member_id: "m1",
  run_id: id("r", 1),
  previous_entry_id: previous,
  entry_id: entryId,
  connector_id: "other",
  manifest_sha256: "1".repeat(64),
  connector_dir_sha256: "2".repeat(64),
  workspace: "/w2",
  session_id: session,
  harness_version: "2",
  tracer_version: "2",
  client_ip: null,
  client_country: null,
  at,
  ...over,
});

const START = "2026-11-06T09:00:00Z";
const DEADLINE = "2026-11-13T09:00:00Z";
const R1 = id("r", 1);
const E1 = id("e", 1);
const G = "0".repeat(64);

test("a run ends at its deadline or when it was replaced, and is closed strictly more than 24 hours after", () => {
  const run = { deadline_at: DEADLINE, replaced_at: null as string | null };
  assert.equal(endsAt(run), DEADLINE);
  assert.equal(endsAt({ ...run, replaced_at: "2026-11-07T09:00:00Z" }), "2026-11-07T09:00:00Z");
  assert.equal(endsAt({ ...run, replaced_at: "2026-11-20T09:00:00Z" }), DEADLINE); // replaced after it was over
  assert.equal(isPastEnd(run, new Date("2026-11-13T08:59:59Z")), false);
  assert.equal(isPastEnd(run, new Date(DEADLINE)), true);
  assert.equal(isPastClose(run, new Date("2026-11-14T09:00:00Z")), false);
  assert.equal(isPastClose(run, new Date("2026-11-14T09:00:01Z")), true);
  assert.equal(isPastClose({ ...run, replaced_at: "2026-11-07T09:00:00Z" }, new Date("2026-11-08T09:00:01Z")), true);
});

test("statusAt: sealed once over with the current entry sealed, incomplete once closed without; stored words stay", () => {
  const run = {
    run_id: R1, event_id: "cup", member_id: "m1", mode: "competition" as const, status: "open" as const, official_start: START,
    deadline_at: DEADLINE, replaced_at: null as string | null, repo_url: "u", first_entry_id: E1, current_entry_id: E1, current_sealed_at: null as string | null,
    stored_bytes: 0, chunk_count: 0,
  };
  const at =(iso: string, over = {}) => statusAt({ ...run, ...over }, new Date(iso));
  assert.equal(at("2026-11-13T08:59:59Z"), "open");
  assert.equal(at("2026-11-13T08:59:59Z", { current_sealed_at: START }), "open"); // sealed, but not over
  assert.equal(at(DEADLINE, { current_sealed_at: START }), "sealed");
  assert.equal(at(DEADLINE), "open");
  assert.equal(at("2026-11-14T09:00:00Z"), "open");
  assert.equal(at("2026-11-14T09:00:01Z"), "incomplete");
  assert.equal(at("2026-11-14T09:00:01Z", { current_sealed_at: START }), "sealed");
  assert.equal(at("2026-11-20T00:00:00Z", { status: "abandoned" }), "abandoned");
  assert.equal(at("2026-11-20T00:00:00Z", { status: "incomplete" }), "incomplete");
});

test("opening a competition run replaces the account's latest one at that instant, and only that one", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [c1, c2, c3] = [await credential(db), await credential(db), await credential(db)];
  assert.equal((await db.openRun(openArgs(1, "s1", START, DEADLINE, c1))).kind, "created");
  assert.equal((await db.openRun(openArgs(2, "s2", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", c2))).kind, "created");
  assert.equal((await db.getRun("m1", id("r", 1), null))?.run.replaced_at, "2026-11-06T09:30:00Z");
  assert.equal((await db.getRun("m1", id("r", 2), null))?.run.replaced_at, null);

  assert.equal((await db.openRun(openArgs(3, "s3", "2026-11-06T10:00:00Z", "2026-11-13T10:00:00Z", c3))).kind, "created");
  assert.equal((await db.getRun("m1", id("r", 1), null))?.run.replaced_at, "2026-11-06T09:30:00Z"); // kept
  assert.equal((await db.getRun("m1", id("r", 2), null))?.run.replaced_at, "2026-11-06T10:00:00Z");
  assert.equal((await db.getRun("m1", id("r", 3), null))?.run.replaced_at, null);

  // Another account's run is not touched.
  const other = await credential(db, "m2");
  assert.equal((await db.openRun(openArgs(4, "s4", "2026-11-06T11:00:00Z", "2026-11-13T11:00:00Z", other, { member_id: "m2" }))).kind, "created");
  assert.equal((await db.getRun("m1", id("r", 3), null))?.run.replaced_at, null);
});

test("test runs replace nothing and are never replaced: an operator's test runs stay open side by side", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [t1, t2, comp, t3, comp2] = [
    await credential(db, "m1", "test"), await credential(db, "m1", "test"), await credential(db), await credential(db, "m1", "test"), await credential(db),
  ];
  const test_ = { mode: "test" as const, event_id: "test" };
  await db.openRun(openArgs(1, "t1", START, DEADLINE, t1, test_));
  await db.openRun(openArgs(2, "t2", "2026-11-06T09:10:00Z", "2026-11-13T09:10:00Z", t2, test_));
  await db.openRun(openArgs(3, "c1", "2026-11-06T09:20:00Z", "2026-11-13T09:20:00Z", comp));
  await db.openRun(openArgs(4, "t3", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", t3, test_));
  for (const n of [1, 2, 3, 4]) assert.equal((await db.getRun("m1", id("r", n), null))?.run.replaced_at, null, `run ${n}`);
  // A competition start replaces the competition run only.
  await db.openRun(openArgs(5, "c2", "2026-11-06T09:40:00Z", "2026-11-13T09:40:00Z", comp2));
  assert.equal((await db.getRun("m1", id("r", 3), null))?.run.replaced_at, "2026-11-06T09:40:00Z");
  for (const n of [1, 2, 4, 5]) assert.equal((await db.getRun("m1", id("r", n), null))?.run.replaced_at, null, `run ${n}`);
});

test("a practice run replaces nothing and is never replaced", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [comp, p1, p2, comp2] = [await credential(db), await credential(db, "m1", "practice"), await credential(db, "m1", "practice"), await credential(db)];
  await db.openRun(openArgs(1, "s1", START, DEADLINE, comp));
  await db.openRun(openArgs(2, "p1", "2026-11-06T09:10:00Z", "2026-11-06T09:20:00Z", p1, { mode: "practice", event_id: "practice" }));
  await db.openRun(openArgs(3, "p2", "2026-11-06T09:11:00Z", "2026-11-06T09:21:00Z", p2, { mode: "practice", event_id: "practice" }));
  for (const n of [1, 2, 3]) assert.equal((await db.getRun("m1", id("r", n), null))?.run.replaced_at, null, `run ${n}`);
  await db.openRun(openArgs(4, "s4", "2026-11-06T09:12:00Z", "2026-11-13T09:12:00Z", comp2));
  assert.equal((await db.getRun("m1", id("r", 1), null))?.run.replaced_at, "2026-11-06T09:12:00Z");
  for (const n of [2, 3]) assert.equal((await db.getRun("m1", id("r", n), null))?.run.replaced_at, null, `run ${n}`);
});

test("the competition start window: its edges are inside, practice has none", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()), { startWindow: { opens: "2026-11-06T08:00:00Z", closes: "2026-11-06T10:00:00Z" } });
  const c = await credential(db);
  const kind = async (n: number, at: string, mode: RunMode = "competition") =>
    (await db.openRun(openArgs(n, `s${n}`, at, DEADLINE, await credential(db, "m1", mode), { mode }))).kind;
  assert.equal(await kind(1, "2026-11-06T07:59:59Z"), "forbidden");
  assert.equal(await kind(2, "2026-11-06T10:00:01Z"), "forbidden");
  assert.equal(await kind(3, "2026-11-06T12:00:00Z", "practice"), "created");
  const late = await db.openRun(openArgs(4, "s4", "2026-11-06T10:00:01Z", DEADLINE, c));
  assert.deepEqual(late, { kind: "forbidden", message: "after the start window you can only `goalcup continue` your run" });
  const early = await db.openRun(openArgs(5, "s5", "2026-11-06T07:00:00Z", DEADLINE, c));
  assert.equal(early.kind, "forbidden");
  assert.equal(await kind(6, "2026-11-06T08:00:00Z"), "created");
  assert.equal(await kind(7, "2026-11-06T10:00:00Z"), "created");

  // A refused start leaves the credential unbound: it can still open a run inside the window.
  assert.equal((await db.openRun(openArgs(8, "s8", "2026-11-06T09:00:00Z", DEADLINE, c))).kind, "created");
});

test("a credential binds in the step that opens the run: a bound or unknown one opens nothing", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [c1, c2] = [await credential(db), await credential(db)];
  const first = await db.openRun(openArgs(1, "s1", START, DEADLINE, c1));
  assert.equal(first.kind, "created");
  assert.deepEqual((await db.credential(c1, START))?.run_id, R1);
  assert.equal((await db.credential(c1, "2026-11-14T08:59:59Z"))?.run_id, R1); // lasts until 24 hours after the deadline
  assert.equal(await db.credential(c1, "2026-11-14T09:00:00Z"), null);

  // The same credential again: conflict, no second run, and the account's run is not replaced.
  assert.deepEqual(await db.openRun(openArgs(2, "s2", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", c1)), { kind: "conflict" });
  assert.equal(await db.getRun("m1", id("r", 2), null), null);
  assert.equal((await db.getRun("m1", R1, null))?.run.replaced_at, null);
  // Unknown credential: the same.
  assert.deepEqual(await db.openRun(openArgs(3, "s3", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", await sha256Hex("gcr_unknown"))), { kind: "conflict" });
  assert.equal(await db.getRun("m1", id("r", 3), null), null);
  // A credential that was not used is still free.
  assert.equal((await db.openRun(openArgs(4, "s4", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", c2))).kind, "created");
});

test("a retry that raced the original open gets that run; another session still conflicts", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const c = await credential(db);
  const first = await db.openRun(openArgs(1, "s1", START, DEADLINE, c));
  assert.equal(first.kind, "created");
  // The same session again, with a run id of its own that must not be used.
  const again = await db.openRun(openArgs(2, "s1", START, DEADLINE, c));
  assert.equal(again.kind, "existing");
  if (again.kind !== "existing") return;
  assert.equal(again.run.run_id, R1);
  assert.equal(again.entry.entry_id, E1);
  assert.equal(await db.getRun("m1", id("r", 2), null), null);
  assert.deepEqual(await db.openRun(openArgs(3, "other", START, DEADLINE, c)), { kind: "conflict" });
  // After a continue, the session of the open call is still segment 0's.
  assert.equal((await db.continueRun(contArgs(id("e", 9), E1, "s-two", START))).kind, "created");
  assert.equal((await db.openRun(openArgs(4, "s1", START, DEADLINE, c))).kind, "existing");
  assert.deepEqual(await db.openRun(openArgs(5, "s-two", START, DEADLINE, c)), { kind: "conflict" });
  assert.equal((await db.getRun("m1", R1, null))?.run.first_entry_id, E1);
});

test("a run is never replaced before it began", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [c1, c2] = [await credential(db), await credential(db)];
  await db.openRun(openArgs(1, "s1", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", c1));
  // Two starts racing: the one that began earlier lands second.
  await db.openRun(openArgs(2, "s2", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z", c2));
  assert.equal((await db.getRun("m1", id("r", 1), null))?.run.replaced_at, "2026-11-06T09:30:00Z");
});

const mk = async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  await db.openRun(openArgs(1, "s1", START, DEADLINE, await credential(db)));
  return db;
};
const hb = (seq: number, head: string, final: boolean, entry = E1) => ({
  member_id: "m1", run_id: R1, entry_id: entry, seq, head, offset_local: 10, final, repo_head: null, at: DEADLINE,
  recording: null, model_requests: null, vitals: null, client_ip: null, client_country: null,
});
const chunk = (first: number, last: number, prev: string, head: string, offset: number, entry = E1, stored_bytes = 7) => ({
  member_id: "m1",
  run_id: R1,
  stored_bytes,
  chunk: {
    entry_id: entry, offset, length: 10, sha256: "1".repeat(64), first_seq: first, last_seq: last, prev, head, r2_key: "k",
    received_at: "2026-11-06T09:30:00Z", ack: { entry_id: entry, seq: last, hash: head, acked_at: "t", server_signature: "s" },
  },
});
const seal = (entry = E1, sealed_at = START, h = "1") => ({
  member_id: "m1", run_id: R1, entry_id: entry, seal_sha256: h, baseline_sha256: "2", seal_key: null, baseline_key: null, sealed_at,
});

test("the final heartbeat commits once; same is existing, different is a conflict", async () => {
  const db = await mk();
  assert.equal(await db.recordHeartbeat(hb(3, "3".repeat(64), false)), "recorded");
  assert.equal((await db.getRun("m1", R1, null))?.entry?.final, null);
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_committed");
  assert.deepEqual((await db.getRun("m1", R1, null))?.entry?.final, { seq: 5, head: "5".repeat(64) });
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_existing");
  assert.equal(await db.recordHeartbeat(hb(6, "6".repeat(64), true)), "final_conflict");
  assert.equal(await db.recordHeartbeat(hb(5, "7".repeat(64), true)), "final_conflict");
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true, id("e", 9))), "not_found"); // an entry that is not this run's
});

test("a final that contradicts the chain already stored is a conflict", async () => {
  const db = await mk();
  assert.equal(await db.commitChunk(chunk(0, 4, G, "4".repeat(64), 0)), "committed");
  assert.equal(await db.recordHeartbeat(hb(3, "3".repeat(64), true)), "final_conflict"); // behind what is stored
  assert.equal(await db.recordHeartbeat(hb(4, "9".repeat(64), true)), "final_conflict"); // same seq, other head
  assert.equal(await db.recordHeartbeat(hb(4, "4".repeat(64), true)), "final_committed");
});

test("commit refuses a chunk past the committed end, and takes one that ends exactly on it", async () => {
  const db = await mk();
  assert.equal(await db.recordHeartbeat(hb(6, "6".repeat(64), true)), "final_committed");
  assert.equal(await db.commitChunk(chunk(0, 7, G, "7".repeat(64), 0)), "beyond_final");
  assert.equal(await db.commitChunk(chunk(0, 6, G, "8".repeat(64), 0)), "beyond_final"); // other head at the final seq
  assert.equal(await db.commitChunk(chunk(0, 6, G, "6".repeat(64), 0)), "committed");
});

test("closeRun: sealed when over with the current entry sealed, incomplete after 24 hours without; then everything is closed", async () => {
  const db = await mk();
  assert.equal(await db.closeRun("m1", R1, "2026-11-13T08:59:59Z"), "open");
  assert.equal(await db.closeRun("m1", R1, DEADLINE), "open"); // over, entry unsealed, inside the 24 hours
  assert.equal(await db.closeRun("m1", R1, "2026-11-14T09:00:00Z"), "open");
  assert.equal(await db.closeRun("m2", R1, "2026-11-20T00:00:00Z"), null); // not this member's
  assert.equal(await db.closeRun("m1", R1, "2026-11-14T09:00:01Z"), "incomplete");
  assert.equal((await db.getRun("m1", R1, null))?.run.status, "incomplete");
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "closed");
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0)), "closed");
  assert.deepEqual(await db.storeReceipt("m1", R1, E1, "b".repeat(64), { claims: { chain_head: "b".repeat(64) } } as never), { kind: "closed" });
  assert.equal(await db.closeRun("m1", R1, "2026-12-20T00:00:00Z"), "incomplete"); // stays

  const sealedFirst = await mk();
  assert.equal(await sealedFirst.recordHeartbeat(hb(0, G, true)), "final_committed");
  assert.equal(await sealedFirst.commitChunk(chunk(0, 0, G, G, 0)), "committed");
  assert.deepEqual(await sealedFirst.sealRun(seal(E1, "2026-11-10T00:00:00Z")), { kind: "sealed", run_status: "open" });
  assert.equal(await sealedFirst.closeRun("m1", R1, "2026-11-12T00:00:00Z"), "open"); // sealed, not yet over
  assert.equal(await sealedFirst.closeRun("m1", R1, DEADLINE), "sealed");
  assert.equal((await sealedFirst.getRun("m1", R1, null))?.run.status, "sealed");
  assert.equal(await sealedFirst.closeRun("m1", R1, "2026-12-20T00:00:00Z"), "sealed");
});

test("a seal needs the committed end and the chain held to it, checked in the step that seals", async () => {
  const db = await mk();
  assert.deepEqual(await db.sealRun(seal()), { kind: "no_final" });
  assert.equal(await db.recordHeartbeat(hb(4, "4".repeat(64), true)), "final_committed");
  assert.deepEqual(await db.sealRun(seal()), { kind: "chain_behind" }); // nothing stored
  assert.equal(await db.commitChunk(chunk(0, 3, G, "3".repeat(64), 0)), "committed");
  assert.deepEqual(await db.sealRun(seal()), { kind: "chain_behind" }); // up to 3 of 4
  assert.equal(await db.commitChunk(chunk(4, 4, "3".repeat(64), "4".repeat(64), 10)), "committed");

  assert.deepEqual(await db.sealRun(seal(E1, "2026-11-06T10:00:00Z")), { kind: "sealed", run_status: "open" }); // before the end
  assert.deepEqual(await db.sealRun(seal(E1, DEADLINE)), { kind: "already_sealed", run_status: "sealed" }); // asked again once over
  assert.deepEqual(await db.sealRun(seal(E1, DEADLINE, "other")), { kind: "conflict" });
  assert.deepEqual(await db.sealRun({ ...seal(), entry_id: id("e", 9) }), { kind: "not_found" });
  assert.equal(await db.commitChunk(chunk(5, 5, "4".repeat(64), "5".repeat(64), 20)), "closed"); // a sealed entry takes nothing
  assert.equal(await db.closeRun("m1", R1, DEADLINE), "sealed");
});

test("heartbeat vitals are stored as given, and a repeat of the committed final heartbeat records nothing", async () => {
  const bucket = new FakeBucket();
  const db = LocalDb.create(asR2(bucket));
  await db.openRun(openArgs(1, "s1", START, DEADLINE, await credential(db)));
  const stored = () => JSON.parse(bucket.map.get(`state/runs/${R1}.json`)!.text) as {
    last_vitals: unknown; disk_paused_at: string | null; entries: { heartbeat_count: number; last_heartbeat: { vitals: unknown } }[];
  };
  const vitals = { counts: { off_ms: 0, agent_paused: "disk", mode: "recording" } };
  assert.equal(await db.recordHeartbeat({ ...hb(0, G, false), vitals }), "recorded");
  assert.deepEqual([stored().entries[0]!.last_heartbeat.vitals, stored().last_vitals, stored().disk_paused_at], [vitals, vitals, null]);
  assert.equal(await db.recordHeartbeat({ ...hb(0, G, true), vitals: { counts_dropped: 4000 } }), "final_committed");
  assert.deepEqual(stored().last_vitals, { counts_dropped: 4000 });
  const rows = stored().entries[0]!.heartbeat_count;
  assert.equal(await db.recordHeartbeat(hb(0, G, true)), "final_existing");
  assert.equal(stored().entries[0]!.heartbeat_count, rows);
});

test("sealing the current entry after the end seals the run in the same step; an older entry does not", async () => {
  const db = await mk();
  const c = await db.continueRun(contArgs(id("e", 2), E1, "s2", "2026-11-06T10:00:00Z"));
  assert.equal(c.kind, "created");
  const E2 = id("e", 2);
  for (const entry of [E1, E2]) {
    assert.equal(await db.recordHeartbeat(hb(0, G, true, entry)), "final_committed");
    assert.equal(await db.commitChunk(chunk(0, 0, G, G, 0, entry)), "committed");
  }
  assert.deepEqual(await db.sealRun(seal(E1, DEADLINE)), { kind: "sealed", run_status: "open" }); // over, but not the current entry
  assert.deepEqual(await db.sealRun(seal(E2, DEADLINE)), { kind: "sealed", run_status: "sealed" });
  assert.equal((await db.getRun("m1", R1, E1))?.run.status, "sealed");
});

test("continue adds a segment after the current entry, with its own chain; the old one keeps taking uploads", async () => {
  const db = await mk();
  const E2 = id("e", 2);
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0)), "committed");
  const made = await db.continueRun(contArgs(E2, E1, "s2", "2026-11-06T10:00:00Z", { client_ip: "198.51.100.7" }));
  assert.equal(made.kind, "created");
  if (made.kind !== "created") return;
  assert.equal(made.entry.segment, 1);
  assert.equal(made.entry.entry_id, E2);
  assert.equal(made.entry.session_id, "s2");
  assert.equal(made.entry.connector_id, "other");
  assert.equal(made.entry.next_seq, 0);
  assert.equal(made.entry.head, G);
  assert.equal(made.run.current_entry_id, E2);
  assert.equal(made.run.official_start, START);
  assert.equal(made.run.deadline_at, DEADLINE);

  // The position of each entry, and the current one by default.
  const current = await db.getRun("m1", R1, null);
  assert.equal(current?.entry?.entry_id, E2);
  const old = await db.getRun("m1", R1, E1);
  assert.equal(old?.entry?.next_seq, 2);
  assert.equal((await db.getRun("m1", R1, id("e", 9)))?.entry, null);
  assert.equal(await db.getRun("m2", R1, E1), null);

  // Each entry takes chunks of its own chain.
  assert.equal(await db.commitChunk(chunk(2, 3, "1".repeat(64), "3".repeat(64), 10, E1)), "committed");
  assert.equal(await db.commitChunk(chunk(0, 5, G, "5".repeat(64), 0, E2)), "committed");
  assert.equal((await db.getChunk("m1", R1, E2, 0))?.last_seq, 5);
  assert.equal((await db.getChunk("m1", R1, E1, 10))?.last_seq, 3);
  assert.equal(await db.getChunk("m1", R1, E2, 10), null);
});

test("stored_bytes and chunk_count add up per committed chunk, over every entry of the run, and a refused commit adds nothing", async () => {
  const db = await mk();
  const totals = async () => {
    const run = (await db.getRun("m1", R1, null))!.run;
    return [run.stored_bytes, run.chunk_count];
  };
  assert.deepEqual(await totals(), [0, 0]);
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0, E1, 1000)), "committed");
  assert.deepEqual(await totals(), [1000, 1]);
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0, E1, 1000)), "conflict"); // the same chunk again: not counted twice
  assert.equal(await db.commitChunk(chunk(2, 3, "1".repeat(64), "3".repeat(64), 10, E1, 2500)), "committed");
  assert.deepEqual(await totals(), [3500, 2]);
  // A second entry counts towards the same run.
  const E2 = id("e", 2);
  assert.equal((await db.continueRun(contArgs(E2, E1, "s2", "2026-11-06T10:00:00Z"))).kind, "created");
  assert.equal(await db.commitChunk(chunk(0, 0, G, "9".repeat(64), 0, E2, 500)), "committed");
  assert.deepEqual(await totals(), [4000, 3]);
  // An entry that is not this run's takes nothing, and adds nothing.
  assert.equal(await db.commitChunk(chunk(9, 9, "x".repeat(64), "y".repeat(64), 99, id("e", 7), 999)), "not_found");
  assert.deepEqual(await totals(), [4000, 3]);
});

test("continue: the same call again is existing, a stale previous is a conflict, an ended run is closed", async () => {
  const db = await mk();
  const [E2, E3] = [id("e", 2), id("e", 3)];
  assert.equal((await db.continueRun(contArgs(E2, E1, "s2", "2026-11-06T10:00:00Z"))).kind, "created");
  const again = await db.continueRun(contArgs(id("e", 8), E1, "s2", "2026-11-06T10:05:00Z"));
  assert.equal(again.kind, "existing");
  assert.equal(again.kind === "existing" ? again.entry.entry_id : "", E2); // the entry that was made, not a new one
  assert.equal((await db.getRun("m1", R1, null))?.entry?.segment, 1);

  assert.deepEqual(await db.continueRun(contArgs(E3, E1, "s3", "2026-11-06T10:10:00Z")), { kind: "conflict" }); // E1 is not the latest
  assert.deepEqual(await db.continueRun(contArgs(E3, id("e", 7), "s3", "2026-11-06T10:10:00Z")), { kind: "conflict" });
  assert.deepEqual(await db.continueRun(contArgs(E3, E2, "s3", "2026-11-06T10:10:00Z", { member_id: "m2" })), { kind: "not_found" });
  assert.equal((await db.continueRun(contArgs(E3, E2, "s3", "2026-11-13T08:59:59Z"))).kind, "created");
  assert.deepEqual(await db.continueRun(contArgs(id("e", 4), E3, "s4", DEADLINE)), { kind: "closed" }); // at the end
  // The same call again after the end is still the answer it was.
  assert.equal((await db.continueRun(contArgs(id("e", 5), E2, "s3", DEADLINE))).kind, "existing");
});

test("continue is closed to a replaced run and to an abandoned one", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [c1, c2] = [await credential(db), await credential(db)];
  await db.openRun(openArgs(1, "s1", START, DEADLINE, c1));
  await db.openRun(openArgs(2, "s2", "2026-11-06T09:30:00Z", "2026-11-13T09:30:00Z", c2)); // replaces run 1 then
  assert.deepEqual(await db.continueRun(contArgs(id("e", 5), E1, "s5", "2026-11-06T09:30:00Z")), { kind: "closed" });
  assert.equal((await db.continueRun(contArgs(id("e", 5), id("e", 2), "s5", "2026-11-06T09:31:00Z", { run_id: id("r", 2) }))).kind, "created");
  const c3 = await credential(db, "m1", "practice");
  await db.openRun(openArgs(3, "s3", START, "2026-11-06T09:10:00Z", c3, { mode: "practice" }));
  assert.equal(await db.abandonRun("m1", id("r", 3), "why", START), "abandoned");
  assert.deepEqual(await db.continueRun(contArgs(id("e", 6), id("e", 3), "s6", START, { run_id: id("r", 3) })), { kind: "closed" });
});

test("abandon is refused once any entry holds a chunk", async () => {
  const db = await mk();
  assert.equal(await db.continueRun(contArgs(id("e", 2), E1, "s2", START)).then((r) => r.kind), "created");
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0, id("e", 2))), "committed");
  assert.equal(await db.abandonRun("m1", R1, "why", START), "has_chunks");
});

// ---- the device flow and the run credential --------------------------------------------------

const CODE = "BCDFGHJK";
const newDevice = async (over: Record<string, unknown> = {}) => ({
  device_sha256: await sha256Hex("device-1"),
  user_sha256: await sha256Hex(CODE),
  purpose: "run" as const,
  mode: "practice" as const,
  connector_id: "codex-cli",
  tracer_version: "1",
  created_at: "2026-11-06T09:00:00Z",
  expires_at: "2026-11-06T09:15:00Z",
  ...over,
});

test("a device request: pending, approved, picked up once, with the credential minted at pickup", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const dev = await newDevice();
  await db.createDevice(dev);
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:01:00Z"), { kind: "pending" });
  assert.equal(await db.approveDevice(CODE, "m1", true, "2026-11-06T09:05:00Z"), "ok");
  assert.equal(await db.approveDevice(CODE, "m1", false, "2026-11-06T09:05:30Z"), "already_decided");

  const got = await db.pollDevice(dev.device_sha256, "2026-11-06T09:06:00Z");
  assert.equal(got.kind, "approved");
  if (got.kind !== "approved") return;
  assert.equal(got.purpose, "run");
  assert.match(got.credential_plain ?? "", /^gcr_[0-9a-f]{64}$/);
  assert.equal(got.expires_at, "2026-11-06T10:05:00Z"); // approved_at + 1 hour
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:06:05Z"), { kind: "expired" }); // once
  assert.deepEqual(await db.pollDevice(await sha256Hex("never-issued"), "2026-11-06T09:06:05Z"), { kind: "expired" });

  // The credential is stored hashed and names the member and the mode, unbound.
  const hash = await sha256Hex(got.credential_plain ?? "");
  assert.deepEqual(await db.credential(hash, "2026-11-06T09:07:00Z"), {
    member_id: "m1", run_id: null, mode: "practice", expires_at: "2026-11-06T10:05:00Z",
  });
});

test("credential expiry: approved+1h unbound, deadline+24h bound", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  await db.createDevice(await newDevice());
  await db.approveDevice(CODE, "m1", true, "2026-11-06T09:05:00Z");
  const got = await db.pollDevice(await sha256Hex("device-1"), "2026-11-06T09:06:00Z");
  const hash = await sha256Hex(got.kind === "approved" ? (got.credential_plain ?? "") : "");

  assert.notEqual(await db.credential(hash, "2026-11-06T10:04:59Z"), null);
  assert.equal(await db.credential(hash, "2026-11-06T10:05:00Z"), null); // unbound: over at approved + 1 h

  const deadline = "2026-11-13T09:06:00Z";
  assert.equal(boundCredentialExpiry(deadline), "2026-11-14T09:06:00Z");
  const opened = await db.openRun(openArgs(1, "s1", "2026-11-06T09:06:00Z", deadline, hash, { mode: "practice" }));
  assert.equal(opened.kind, "created");
  const bound = await db.credential(hash, "2026-11-14T09:05:59Z");
  assert.deepEqual(bound, { member_id: "m1", run_id: R1, mode: "practice", expires_at: "2026-11-14T09:06:00Z" });
  assert.equal(await db.credential(hash, "2026-11-14T09:06:00Z"), null); // bound: over at deadline + 24 h
  assert.equal(await db.credential(await sha256Hex("gcr_unknown"), "2026-11-06T09:07:00Z"), null);
});

test("a request that is not decided in time is expired; a denial is 403 until then", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const dev = await newDevice();
  await db.createDevice(dev);
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:14:59Z"), { kind: "pending" });
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:15:00Z"), { kind: "expired" });
  assert.equal(await db.approveDevice(CODE, "m1", true, "2026-11-06T09:15:00Z"), "expired");
  assert.equal(await db.approveDevice("BBBBBBBB", "m1", true, "2026-11-06T09:01:00Z"), "not_found");

  const other = await newDevice({ device_sha256: await sha256Hex("device-2"), user_sha256: await sha256Hex("CCCCCCCC") });
  await db.createDevice(other);
  assert.equal(await db.approveDevice("CCCCCCCC", "m1", false, "2026-11-06T09:02:00Z"), "ok");
  assert.deepEqual(await db.pollDevice(other.device_sha256, "2026-11-06T09:03:00Z"), { kind: "denied" });
  assert.deepEqual(await db.pollDevice(other.device_sha256, "2026-11-06T09:14:00Z"), { kind: "denied" });
  assert.deepEqual(await db.pollDevice(other.device_sha256, "2026-11-06T09:15:00Z"), { kind: "expired" });
});

test("a login approval hands back the handle once and mints no credential", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const dev = await newDevice({ purpose: "login" });
  await db.createDevice(dev);
  await db.approveDevice(CODE, "m1", true, "2026-11-06T09:05:00Z");
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:06:00Z"), { kind: "approved", purpose: "login", handle: "m1" });
  assert.deepEqual(await db.pollDevice(dev.device_sha256, "2026-11-06T09:06:01Z"), { kind: "expired" });
});

// ---- what the cron reads and writes ----------------------------------------------------------

const beat = (at: string, over: Partial<HeartbeatArgs> = {}, entry = E1): HeartbeatArgs => ({ ...hb(0, G, false, entry), at, ...over });

test("last_heartbeat_at belongs to the run: a heartbeat of any entry sets it", async () => {
  const db = await mk();
  const two = await db.continueRun(contArgs(id("e", 2), E1, "s2", "2026-11-06T10:00:00Z"));
  assert.equal(two.kind, "created");
  const tick = async () => (await db.runsForTick("2026-11-06T12:00:00Z"))[0]!;
  assert.equal((await tick()).last_heartbeat_at, null);
  await db.recordHeartbeat(beat("2026-11-06T10:05:00Z", {}, id("e", 2)));
  assert.equal((await tick()).last_heartbeat_at, "2026-11-06T10:05:00Z");
  await db.recordHeartbeat(beat("2026-11-06T10:10:00Z", {}, E1)); // the older entry still beats
  assert.equal((await tick()).last_heartbeat_at, "2026-11-06T10:10:00Z");
  // A heartbeat that was refused (a different end already committed) is not one.
  assert.equal(await db.recordHeartbeat(beat("2026-11-06T10:20:00Z", { seq: 3, head: "3".repeat(64), final: true })), "final_committed");
  assert.equal(await db.recordHeartbeat(beat("2026-11-06T11:00:00Z", { seq: 4, head: "4".repeat(64), final: true })), "final_conflict");
  assert.equal((await tick()).last_heartbeat_at, "2026-11-06T10:20:00Z");
});

test("heartbeat signals: the first gaps or failed flags the run, model_requests only moves up", async () => {
  const db = await mk();
  const tick = async () => (await db.runsForTick("2026-11-06T12:00:00Z"))[0]!;
  const first = await tick();
  assert.deepEqual([first.flagged_at, first.flag_reason, first.last_model_request_at], [null, null, null]); // nothing reported yet
  await db.recordHeartbeat(beat("2026-11-06T09:10:00Z", { recording: "ok", model_requests: 0 }));
  await db.recordHeartbeat(beat("2026-11-06T09:20:00Z", { recording: "failed", model_requests: 4 }));
  await db.recordHeartbeat(beat("2026-11-06T09:30:00Z", { recording: "gaps", model_requests: 4 }));
  await db.recordHeartbeat(beat("2026-11-06T09:40:00Z", { model_requests: 3 }));
  const run = await tick();
  assert.deepEqual([run.flagged_at, run.flag_reason, run.last_model_request_at], ["2026-11-06T09:20:00Z", "failed", "2026-11-06T09:20:00Z"]);
});

test("runsForTick: open runs and recently ended ones, with the run's own facts; not abandoned or long closed", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const [c1, c2, c3] = [await credential(db, "m1"), await credential(db, "m2"), await credential(db, "m3", "practice")];
  await db.openRun(openArgs(1, "s1", START, DEADLINE, c1));
  await db.openRun(openArgs(2, "s2", START, "2026-11-06T10:00:00Z", c2, { member_id: "m2" }));
  await db.openRun(openArgs(3, "s3", START, "2026-11-06T09:10:00Z", c3, { member_id: "m3", mode: "practice" }));
  const ids = async (at: string) => (await db.runsForTick(at)).map((r) => r.run_id);
  assert.deepEqual(await ids(START), [id("r", 1), id("r", 2), id("r", 3)]);
  const mine = (await db.runsForTick(START))[0]!;
  assert.deepEqual([mine.mode, mine.member_id, mine.repo_url, mine.final_commit], ["competition", "m1", "https://github.com/a/b", null]);
  // Closed and ended more than two days ago: left alone. An open one is always in.
  assert.equal(await db.closeRun("m3", id("r", 3), "2026-11-07T10:00:00Z"), "incomplete");
  assert.deepEqual(await ids("2026-11-08T09:10:00Z"), [id("r", 1), id("r", 2), id("r", 3)]); // 48 h after the practice run ended
  assert.deepEqual(await ids("2026-11-08T09:10:01Z"), [id("r", 1), id("r", 2)]);
  assert.deepEqual(await db.abandonRun("m1", id("r", 1), "gone", START), "abandoned");
  assert.deepEqual(await ids(START), [id("r", 2), id("r", 3)]);
});

test("queueNotices writes a notice once per run, kind and key, and counts what it wrote", async () => {
  const bucket = new FakeBucket();
  const db = LocalDb.create(asR2(bucket));
  const n = { run_id: R1, member_id: "m1", kind: "silence" as const, dedupe_key: START, payload: { last_heartbeat_at: START }, at: "2026-11-06T09:21:00Z" };
  assert.equal(await db.queueNotices([]), 0);
  assert.equal(await db.queueNotices([n, { ...n, payload: { twice: true } }]), 1); // the same key twice in one batch
  assert.equal(await db.queueNotices([{ ...n, payload: { other: true } }]), 0); // the first stays
  assert.equal(
    await db.queueNotices([
      n,
      { ...n, dedupe_key: "2026-11-06T09:30:00Z" },
      { ...n, kind: "started", dedupe_key: "" },
      { ...n, run_id: id("r", 2), dedupe_key: "", kind: "started" },
    ]),
    3,
  );
  const stored = [...bucket.map.entries()].filter(([k]) => k.startsWith("state/notices/")).map(([, v]) => JSON.parse(v.text));
  assert.equal(stored.length, 4);
  assert.deepEqual(stored.find((x) => x.kind === "silence" && x.dedupe_key === START), {
    run_id: R1, member_id: "m1", kind: "silence", dedupe_key: START, payload: { last_heartbeat_at: START }, created_at: "2026-11-06T09:21:00Z", sent_at: null,
  });
});

test("recordFinalCommit keeps the first commit it was given", async () => {
  const db = await mk();
  await db.recordFinalCommit("m1", R1, "a".repeat(40), DEADLINE);
  await db.recordFinalCommit("m1", R1, "b".repeat(40), DEADLINE);
  await db.recordFinalCommit("m2", R1, "c".repeat(40), DEADLINE); // not this member's
  assert.equal((await db.runsForTick(DEADLINE))[0]!.final_commit, "a".repeat(40));
});
