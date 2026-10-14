// Configuration guards and the SupabaseDb request shape (against a stubbed fetch: no Supabase here).

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { getDb, readConfig } from "../src/context.ts";
import { SupabaseDb } from "../src/db-supabase.ts";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import { REPO_URL, parseSafeUint } from "../src/util.ts";

const baseEnv = { EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;

/** Run `fn` with console.error captured; returns what was logged. */
async function captureErrors(fn: () => Promise<unknown>): Promise<string> {
  const lines: string[] = [];
  const real = console.error;
  console.error = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    await fn();
  } catch {
    // the caller asserts on the rejection separately
  } finally {
    console.error = real;
  }
  return lines.join("\n");
}

test("the default database backend is supabase, and it refuses to start without its secrets", async () => {
  const logged = await captureErrors(() => getDb({ ...baseEnv }));
  assert.match(logged, /SUPABASE_URL/); // the detail is in the log...
  await assert.rejects(getDb({ ...baseEnv }), (e) => e instanceof ApiError && e.code === "misconfigured" && !/SUPABASE/.test(e.message)); // ...not in the answer
});

test("the local backend needs nothing else to start", async () => {
  await getDb({ ...baseEnv, DB_BACKEND: "local" });
});

test("an unknown backend is refused", async () => {
  await captureErrors(async () => {
    await assert.rejects(getDb({ ...baseEnv, DB_BACKEND: "sqlite" }), ApiError);
  });
});

test("config defaults and validation", () => {
  assert.deepEqual(readConfig(baseEnv), { eventId: "practice", runSeconds: 168 * 3600, abandonAfterSeconds: 86400, githubCheck: false });
  assert.equal(readConfig({ ...baseEnv, RUN_HOURS: "2" }).runSeconds, 7200);
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "on" }).githubCheck, true);
  assert.equal(readConfig({ ...baseEnv, GITHUB_CHECK: "yes" }).githubCheck, false);
  assert.throws(() => readConfig({ ...baseEnv, EVENT_ID: "../x" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "0" }), ApiError);
  assert.throws(() => readConfig({ ...baseEnv, RUN_HOURS: "abc" }), ApiError);
});

test("RUN_SECONDS and ABANDON_AFTER_SECONDS are honoured only with the local backend", () => {
  const local = { ...baseEnv, DB_BACKEND: "local" };
  assert.equal(readConfig({ ...local, RUN_SECONDS: "3" }).runSeconds, 3);
  assert.equal(readConfig({ ...local, RUN_SECONDS: "3", RUN_HOURS: "1" }).runSeconds, 3);
  assert.equal(readConfig({ ...local, ABANDON_AFTER_SECONDS: "5" }).abandonAfterSeconds, 5);
  // On any other backend their presence is a mistake, so the Worker refuses rather than ignoring them.
  for (const name of ["RUN_SECONDS", "ABANDON_AFTER_SECONDS"] as const) {
    assert.throws(() => readConfig({ ...baseEnv, [name]: "3" }), (e) => e instanceof ApiError && e.code === "misconfigured");
    assert.throws(() => readConfig({ ...baseEnv, DB_BACKEND: "supabase", [name]: "3" }), ApiError);
  }
  assert.throws(() => readConfig({ ...local, RUN_SECONDS: "0" }), ApiError);
  assert.throws(() => readConfig({ ...local, RUN_SECONDS: "x" }), ApiError);
});

test("TEST_APPROVAL_KEY is honoured only with the local backend or STAGING=on", () => {
  const key = { ...baseEnv, TEST_APPROVAL_KEY: "k" };
  readConfig({ ...key, DB_BACKEND: "local" });
  readConfig({ ...key, STAGING: "on" });
  readConfig({ ...key, DB_BACKEND: "supabase", STAGING: "on" });
  readConfig(baseEnv); // no key, nothing to refuse
  readConfig({ ...baseEnv, STAGING: "on" });
  for (const env of [key, { ...key, DB_BACKEND: "supabase" }, { ...key, STAGING: "yes" }, { ...key, STAGING: "off" }]) {
    assert.throws(() => readConfig(env), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
  }
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
    "https://github.com/a/..",
    "https://github.com/a/.",
    "https://github.com/a_b/c",
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
  const cred = { member_id: "member-uuid", run_id: null, mode: "practice", expires_at: "2026-11-06T10:05:00Z" };
  const calls = stubFetch(() => new Response(JSON.stringify(cred), { status: 200, headers: { "content-type": "application/json" } }));
  const db = new SupabaseDb("https://proj.supabase.example/", "service-key");
  assert.deepEqual(await db.credential("ab".repeat(32), "2026-11-06T09:06:00Z"), cred);
  const c = calls[0]!;
  assert.equal(c.url, "https://proj.supabase.example/rest/v1/rpc/upload_credential");
  assert.equal(c.init.method, "POST");
  const h = c.init.headers as Record<string, string>;
  assert.equal(h["content-profile"], "goalcup");
  assert.equal(h["accept-profile"], "goalcup");
  assert.equal(h.apikey, "service-key");
  assert.equal(h.authorization, "Bearer service-key");
  assert.deepEqual(JSON.parse(c.init.body as string), { p_credential_hash: "ab".repeat(32), p_now: "2026-11-06T09:06:00Z" });
});

test("SupabaseDb: unknown credential is null; errors become 502; odd answers become 502", async () => {
  const db = new SupabaseDb("https://p.example", "k");
  stubFetch(() => new Response("null", { status: 200 }));
  assert.equal(await db.credential("x", "t"), null);
  stubFetch(() => new Response('{"message":"boom"}', { status: 500 }));
  await assert.rejects(db.credential("x", "t"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => new Response('"surprise"', { status: 200 }));
  await assert.rejects(db.abandonRun("m", "r0000000000000000", "why", "2026-11-06T09:00:00Z"), (e) => e instanceof ApiError && e.status === 502);
  stubFetch(() => {
    throw new TypeError("network down");
  });
  await assert.rejects(db.credential("x", "t"), (e) => e instanceof ApiError && e.status === 502);
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
