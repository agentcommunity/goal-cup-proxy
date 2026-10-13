// Configuration guards and the SupabaseDb request shape (against a stubbed fetch: no Supabase here).

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { getDb, readConfig } from "../src/context.ts";
import { SupabaseDb } from "../src/db-supabase.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import { REPO_URL, parseSafeUint } from "../src/util.ts";

const baseEnv = { EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;

test("the default database backend is supabase, and it refuses to start without its secrets", async () => {
  await assert.rejects(getDb({ ...baseEnv }), (e) => e instanceof ApiError && e.code === "misconfigured" && /SUPABASE_URL/.test(e.message));
});

test("the local backend refuses to start without DEV_TOKENS", async () => {
  await assert.rejects(
    getDb({ ...baseEnv, DB_BACKEND: "local" }),
    (e) => e instanceof ApiError && e.code === "misconfigured" && /DEV_TOKENS/.test(e.message),
  );
  await assert.rejects(getDb({ ...baseEnv, DB_BACKEND: "local", DEV_TOKENS: "  " }), ApiError);
  await assert.rejects(getDb({ ...baseEnv, DB_BACKEND: "local", DEV_TOKENS: "no-colon" }), ApiError);
  await getDb({ ...baseEnv, DB_BACKEND: "local", DEV_TOKENS: "t1:m1,t2:m2" }); // fine
});

test("an unknown backend is refused", async () => {
  await assert.rejects(getDb({ ...baseEnv, DB_BACKEND: "sqlite" }), ApiError);
});

test("config defaults and validation", () => {
  assert.deepEqual(readConfig(baseEnv), { eventId: "practice", runHours: 168, graceHours: 48, githubCheck: false });
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "on" }).githubCheck, true);
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "yes" }).githubCheck, false);
  assert.throws(() => readConfig({ ...baseEnv, EVENT_ID: "../x" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "0" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "abc" }), ApiError);
  assert.equal(readConfig({ ...baseEnv, GRACE_HOURS: "0" }).graceHours, 0);
});

test("repo_url shape", () => {
  assert.ok(REPO_URL.test("https://github.com/agentcommunity/goal-cup-tracer"));
  assert.ok(REPO_URL.test("https://github.com/a-b/c.d_e"));
  for (const bad of [
    "http://github.com/a/b",
    "https://github.com/a",
    "https://github.com/a/b/",
    "https://github.com/a/b/tree/main",
    "https://github.com/a/b?x=1",
    "https://gitlab.com/a/b",
    "https://github.com/a/b c",
    "https://github.com.evil.example/a/b",
  ]) {
    assert.equal(REPO_URL.test(bad), false, bad);
  }
});

test("parseSafeUint is strict", () => {
  assert.equal(parseSafeUint("0"), 0);
  assert.equal(parseSafeUint("123"), 123);
  for (const bad of ["", "01", "-1", "1.5", "1e3", " 1", "99999999999999999", "0x10"]) assert.equal(parseSafeUint(bad), null, bad);
});

// ---- SupabaseDb against a stubbed fetch ---------------------------------------------------

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

function stubFetch(reply: (url: string, init: RequestInit) => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, init: init ?? {} });
    return reply(url, init ?? {});
  }) as typeof fetch;
  return calls;
}

test("SupabaseDb calls PostgREST RPC in schema goalcup with the dedicated key", async () => {
  const calls = stubFetch(() => new Response(JSON.stringify("member-uuid"), { status: 200, headers: { "content-type": "application/json" } }));
  const db = new SupabaseDb("https://proj.supabase.example/", "service-key");
  assert.equal(await db.memberForToken("ab".repeat(32)), "member-uuid");
  const c = calls[0]!;
  assert.equal(c.url, "https://proj.supabase.example/rest/v1/rpc/upload_lookup_token");
  assert.equal(c.init.method, "POST");
  const h = c.init.headers as Record<string, string>;
  assert.equal(h["content-profile"], "goalcup");
  assert.equal(h["accept-profile"], "goalcup");
  assert.equal(h.apikey, "service-key");
  assert.equal(h.authorization, "Bearer service-key");
  assert.deepEqual(JSON.parse(c.init.body as string), { p_token_hash: "ab".repeat(32) });
});

test("SupabaseDb: unknown token is null; errors become 502; odd answers become 502", async () => {
  const db = new SupabaseDb("https://p.example", "k");
  stubFetch(() => new Response("null", { status: 200 }));
  assert.equal(await db.memberForToken("x"), null);
  stubFetch(() => new Response('{"message":"boom"}', { status: 500 }));
  await assert.rejects(db.memberForToken("x"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => new Response('"surprise"', { status: 200 }));
  await assert.rejects(db.abandonRun("m", "r0000000000000000", "why", "2026-11-06T09:00:00Z"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => {
    throw new TypeError("network down");
  });
  await assert.rejects(db.memberForToken("x"), (e) => e instanceof ApiError && e.status === 502);
});

test("SupabaseDb maps the open_run answers", async () => {
  const run = { run_id: "r0123456789abcdef", entry_id: "e0123456789abcdef" };
  const db = new SupabaseDb("https://p.example", "k");
  const args = {
    member_id: "m", event_id: "practice", run_id: "r1", entry_id: "e1", official_start: "s", deadline_at: "d", connector_id: "c",
    manifest_sha256: "a", connector_dir_sha256: "b", workspace: "w", session_id: "s", harness_version: "h", tracer_version: "t", repo_url: "u",
  };
  stubFetch(() => new Response(JSON.stringify({ result: "created", run }), { status: 200 }));
  assert.deepEqual(await db.openRun(args), { kind: "created", run });
  stubFetch(() => new Response(JSON.stringify({ result: "existing", run }), { status: 200 }));
  assert.equal((await db.openRun(args)).kind, "existing");
  stubFetch(() => new Response(JSON.stringify({ result: "conflict" }), { status: 200 }));
  assert.deepEqual(await db.openRun(args), { kind: "conflict" });
  stubFetch(() => new Response(JSON.stringify({ result: "forbidden", message: "Accept the terms first." }), { status: 200 }));
  assert.deepEqual(await db.openRun(args), { kind: "forbidden", message: "Accept the terms first." });
});
