// LocalDb on an in-memory bucket: the rules that need a clock the HTTP tests cannot move.

import assert from "node:assert/strict";
import { test } from "node:test";
import { boundCredentialExpiry, isAbandonable } from "../src/db.ts";
import type { OpenRunArgs } from "../src/db.ts";
import { LocalDb } from "../src/db-local.ts";
import { sha256Hex } from "../src/util.ts";
import { FakeBucket, asR2 } from "./helpers/fake-r2.ts";

const open = (id: number, session: string, startedAt: string, deadlineAt: string): OpenRunArgs => ({
  member_id: "m1",
  event_id: "practice",
  run_id: `r${String(id).padStart(16, "0")}`,
  entry_id: `e${String(id).padStart(16, "0")}`,
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
});

test("isAbandonable: strictly more than 24 hours after the deadline", () => {
  const deadline = "2026-11-13T09:00:00Z";
  assert.equal(isAbandonable(deadline, new Date("2026-11-14T09:00:00Z")), false);
  assert.equal(isAbandonable(deadline, new Date("2026-11-14T09:00:01Z")), true);
  assert.equal(isAbandonable(deadline, new Date("2026-11-06T09:00:00Z")), false);
});

test("opening a run abandons the account's unsealed run that is 24 hours past its deadline", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const first = await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  assert.equal(first.kind, "created");

  // Exactly 24 hours past the deadline: still the account's open run.
  assert.equal((await db.openRun(open(2, "s2", "2026-11-14T09:00:00Z", "2026-11-21T09:00:00Z"))).kind, "conflict");

  // One second later it is abandoned and the slot is free.
  const next = await db.openRun(open(3, "s2", "2026-11-14T09:00:01Z", "2026-11-21T09:00:01Z"));
  assert.equal(next.kind, "created");
  const old = await db.getRun("m1", "r0000000000000001");
  assert.equal(old?.status, "abandoned");
  assert.equal((await db.getRun("m1", "r0000000000000003"))?.status, "open");
});

const mk = async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  return db;
};
const hb = (seq: number, head: string, final: boolean) => ({
  member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seq, head, offset_local: 10, final, repo_head: null, at: "2026-11-13T09:00:00Z",
});
const chunk = (first: number, last: number, prev: string, head: string, offset: number) => ({
  member_id: "m1",
  run_id: "r0000000000000001",
  chunk: {
    entry_id: "e0000000000000001", offset, length: 10, sha256: "1".repeat(64), first_seq: first, last_seq: last, prev, head, r2_key: "k",
    received_at: "2026-11-06T09:30:00Z", ack: { entry_id: "e0000000000000001", seq: last, hash: head, acked_at: "t", server_signature: "s" },
  },
});
const G = "0".repeat(64);

test("the final heartbeat commits once; same is existing, different is a conflict", async () => {
  const db = await mk();
  assert.equal(await db.recordHeartbeat(hb(3, "3".repeat(64), false)), "recorded");
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.final, null);
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_committed");
  assert.deepEqual((await db.getRun("m1", "r0000000000000001"))?.final, { seq: 5, head: "5".repeat(64) });
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_existing");
  assert.equal(await db.recordHeartbeat(hb(6, "6".repeat(64), true)), "final_conflict");
  assert.equal(await db.recordHeartbeat(hb(5, "7".repeat(64), true)), "final_conflict");
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

test("expireRun: only an open, unsealed run past 24 hours, and then heartbeats are closed", async () => {
  const db = await mk();
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-14T09:00:00Z"), false);
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-14T09:00:01Z"), true);
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.status, "abandoned");
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-15T09:00:00Z"), false);
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "closed");
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0)), "closed");
});

test("a final heartbeat is still taken on a sealed run, an ordinary one is a no-op", async () => {
  const db = await mk();
  await db.sealRun({ member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seal_sha256: "1", baseline_sha256: "2", seal_key: "k", baseline_key: "k2", sealed_at: "t" });
  assert.equal(await db.recordHeartbeat(hb(0, G, false)), "recorded");
  assert.equal(await db.recordHeartbeat(hb(-1 + 1, G, true)), "final_committed");
});

test("a sealed run never blocks, and an old run of a different member is untouched", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  const a = await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  assert.equal(a.kind, "created");
  const sealed = await db.sealRun({
    member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seal_sha256: "1", baseline_sha256: "2",
    seal_key: "k", baseline_key: "k2", sealed_at: "2026-11-13T09:00:00Z",
  });
  assert.equal(sealed, "sealed");
  assert.equal((await db.openRun(open(2, "s2", "2026-11-13T10:00:00Z", "2026-11-20T10:00:00Z"))).kind, "created");
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.status, "sealed");
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
  await db.bindCredential(hash, "r0000000000000001", boundCredentialExpiry(deadline));
  const bound = await db.credential(hash, "2026-11-14T09:05:59Z");
  assert.deepEqual(bound, { member_id: "m1", run_id: "r0000000000000001", mode: "practice", expires_at: "2026-11-14T09:06:00Z" });
  assert.equal(await db.credential(hash, "2026-11-14T09:06:00Z"), null); // bound: over at deadline + 24 h
  assert.equal(await db.credential(await sha256Hex("gcr_unknown"), "2026-11-06T09:07:00Z"), null);
});

test("a credential binds once: the same run again is fine, another run or an unknown credential is a conflict", async () => {
  const db = LocalDb.create(asR2(new FakeBucket()));
  await db.createDevice(await newDevice());
  await db.approveDevice(CODE, "m1", true, "2026-11-06T09:05:00Z");
  const got = await db.pollDevice(await sha256Hex("device-1"), "2026-11-06T09:06:00Z");
  const hash = await sha256Hex(got.kind === "approved" ? (got.credential_plain ?? "") : "");
  const until = "2026-11-14T09:00:00Z";

  assert.equal(await db.bindCredential(hash, "r0000000000000001", until), "bound");
  assert.equal(await db.bindCredential(hash, "r0000000000000001", until), "bound"); // a lost reply, asked again
  assert.equal(await db.bindCredential(hash, "r0000000000000002", "2026-12-01T00:00:00Z"), "conflict");
  assert.deepEqual(await db.credential(hash, "2026-11-07T00:00:00Z"), { member_id: "m1", run_id: "r0000000000000001", mode: "practice", expires_at: until });
  assert.equal(await db.bindCredential(await sha256Hex("gcr_unknown"), "r0000000000000001", until), "conflict");
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
