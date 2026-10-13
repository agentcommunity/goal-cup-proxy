// The GitHub public-repo check (GITHUB_CHECK=on), against a stubbed fetch: no network.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import type { Ctx } from "../src/context.ts";
import type { Db, OpenRunArgs } from "../src/db.ts";
import { ApiError } from "../src/errors.ts";
import { openRun } from "../src/runs.ts";
import { openBody } from "./helpers/client.ts";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function ctxWith(githubCheck: boolean, token?: string) {
  const opened: OpenRunArgs[] = [];
  const db = {
    async openRun(a: OpenRunArgs) {
      opened.push(a);
      return { kind: "created" as const, run: { run_id: a.run_id, entry_id: a.entry_id, official_start: a.official_start, deadline_at: a.deadline_at } };
    },
  } as unknown as Db;
  const ctx = {
    env: { GITHUB_TOKEN: token },
    db,
    cfg: { eventId: "practice", runSeconds: 168 * 3600, abandonAfterSeconds: 86400, githubCheck },
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
  const res = await openRun(ctx, "m", req());
  assert.equal(res.status, 201);
  assert.equal(calls.length, 0);
  assert.equal(opened[0]!.official_start, "2026-11-06T09:00:00Z");
  assert.equal(opened[0]!.deadline_at, "2026-11-13T09:00:00Z");
});

test("on: a public repository passes, and the token is sent when configured", async () => {
  const calls = stubGithub(() => new Response(JSON.stringify({ private: false }), { status: 200 }));
  const { ctx } = ctxWith(true, "ghp_example");
  assert.equal((await openRun(ctx, "m", req())).status, 201);
  assert.equal(calls[0]!.url, "https://api.github.com/repos/someone/goal-cup-entry");
  assert.equal(calls[0]!.headers.authorization, "Bearer ghp_example");
});

test("on: private, missing, and unreachable repositories are refused", async () => {
  const { ctx, opened } = ctxWith(true);
  stubGithub(() => new Response(JSON.stringify({ private: true }), { status: 200 }));
  await assert.rejects(openRun(ctx, "m", req()), (e) => e instanceof ApiError && e.status === 400 && e.code === "malformed");
  stubGithub(() => new Response("{}", { status: 404 }));
  await assert.rejects(openRun(ctx, "m", req()), (e) => e instanceof ApiError && e.status === 400);
  stubGithub(() => new Response("{}", { status: 403 })); // e.g. API rate limit: our problem, so retryable
  await assert.rejects(openRun(ctx, "m", req()), (e) => e instanceof ApiError && e.status === 502);
  globalThis.fetch = (async () => {
    throw new TypeError("offline");
  }) as typeof fetch;
  await assert.rejects(openRun(ctx, "m", req()), (e) => e instanceof ApiError && e.status === 502);
  assert.equal(opened.length, 0, "no run is created when the check fails");
});
