// LocalDb on an in-memory bucket: the rules that need a clock the HTTP tests cannot move.

import assert from "node:assert/strict";
import { test } from "node:test";
import { isAbandonable } from "../src/db.ts";
import type { OpenRunArgs } from "../src/db.ts";
import { LocalDb } from "../src/db-local.ts";
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
  const db = await LocalDb.create(asR2(new FakeBucket()), "t1:m1");
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

test("a sealed run never blocks, and an old run of a different member is untouched", async () => {
  const db = await LocalDb.create(asR2(new FakeBucket()), "t1:m1,t2:m2");
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
