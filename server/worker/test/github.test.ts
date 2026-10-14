// The GitHub public-repo check (GITHUB_CHECK=on), against a stubbed fetch: no network.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import type { Auth } from "../src/auth.ts";
import type { Ctx } from "../src/context.ts";
import type { Db, OpenRunArgs } from "../src/db.ts";
import { ApiError } from "../src/errors.ts";
import { openRun } from "../src/runs.ts";
import { openBody } from "./helpers/client.ts";

const AUTH: Auth = { member: "m", credential: "c".repeat(64), run_id: null, mode: "competition" };

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function ctxWith(githubCheck: boolean, token?: string, result: "created" | "conflict" = "created") {
  const opened: OpenRunArgs[] = [];
  const db = {
    async openRun(a: OpenRunArgs) {
      opened.push(a);
      if (result === "conflict") return { kind: "conflict" as const };
      const run = { run_id: a.run_id, official_start: a.official_start, deadline_at: a.deadline_at };
      return { kind: "created" as const, run, entry: { entry_id: a.entry_id } };
    },
  } as unknown as Db;
  const ctx = {
    env: { GITHUB_TOKEN: token },
    db,
    cfg: { eventId: "cup", practiceEventId: "practice", runSeconds: 168 * 3600, practiceRunSeconds: 600, closeAfterSeconds: 86400, startWindow: undefined, githubCheck },
    now: () => new Date("2026-11-06T09:00:00Z"),
  } as unknown as Ctx;
  return { ctx, opened };
}

const req = () => new Request("https://x.example/v1/runs", { method: "POST", body: JSON.stringify(openBody()) });

function stubGithub(reply: () => Response) {
  const calls: { url: string; headers: Record<string, string> }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), headers: (init?.headers ?? {}) as Record<string, string> });
    return reply();
  }) as typeof fetch;
  return calls;
}

test("off: GitHub is never asked", async () => {
  const calls = stubGithub(() => new Response("{}", { status: 500 }));
  const { ctx, opened } = ctxWith(false);
  const res = await openRun(ctx, AUTH, req());
  assert.equal(res.status, 201);
  // The credential is bound to the new run, in the step that opens it, until 24 hours after its deadline.
  assert.equal(opened[0]!.credential, AUTH.credential);
  assert.equal(opened[0]!.credential_expires_at, "2026-11-14T09:00:00Z");
  assert.equal(calls.length, 0);
  assert.equal(opened[0]!.official_start, "2026-11-06T09:00:00Z");
  assert.equal(opened[0]!.deadline_at, "2026-11-13T09:00:00Z");
  assert.equal(opened[0]!.event_id, "cup");
  assert.equal(opened[0]!.mode, "competition");
});

test("a practice credential opens a practice run: the practice event, the practice length", async () => {
  const { ctx, opened } = ctxWith(false);
  assert.equal((await openRun(ctx, { ...AUTH, mode: "practice" }, req())).status, 201);
  assert.equal(opened[0]!.event_id, "practice");
  assert.equal(opened[0]!.mode, "practice");
  assert.equal(opened[0]!.deadline_at, "2026-11-06T09:10:00Z");
  assert.equal(opened[0]!.credential_expires_at, "2026-11-07T09:10:00Z");
});

test("the connection's facts go to the database as given, nulls when absent", async () => {
  const { ctx, opened } = ctxWith(false);
  await openRun(ctx, AUTH, req());
  assert.deepEqual([opened[0]!.client_ip, opened[0]!.client_country, opened[0]!.machine], [null, null, null]);
});

test("on: a public repository passes, and the token is sent when configured", async () => {
  const calls = stubGithub(() => new Response(JSON.stringify({ private: false }), { status: 200 }));
  const { ctx } = ctxWith(true, "ghp_example");
  assert.equal((await openRun(ctx, AUTH, req())).status, 201);
  assert.equal(calls[0]!.url, "https://api.github.com/repos/someone/goal-cup-entry");
  assert.equal(calls[0]!.headers.authorization, "Bearer ghp_example");
});

test("on: private, missing, and unreachable repositories are refused", async () => {
  const { ctx, opened } = ctxWith(true);
  stubGithub(() => new Response(JSON.stringify({ private: true }), { status: 200 }));
  await assert.rejects(openRun(ctx, AUTH, req()), (e) => e instanceof ApiError && e.status === 400 && e.code === "malformed");
  stubGithub(() => new Response("{}", { status: 404 }));
  await assert.rejects(openRun(ctx, AUTH, req()), (e) => e instanceof ApiError && e.status === 400);
  stubGithub(() => new Response("{}", { status: 403 })); // e.g. API rate limit: our problem, so retryable
  await assert.rejects(openRun(ctx, AUTH, req()), (e) => e instanceof ApiError && e.status === 502);
  globalThis.fetch = (async () => {
    throw new TypeError("offline");
  }) as typeof fetch;
  await assert.rejects(openRun(ctx, AUTH, req()), (e) => e instanceof ApiError && e.status === 502);
  assert.equal(opened.length, 0, "no run is created when the check fails");
});

test("a credential that cannot bind to the new run is 409, not a second run for it", async () => {
  stubGithub(() => new Response("{}", { status: 500 }));
  const { ctx } = ctxWith(false, undefined, "conflict");
  await assert.rejects(openRun(ctx, AUTH, req()), (e) => e instanceof ApiError && e.status === 409 && /goalcup continue/.test(e.message));
});
