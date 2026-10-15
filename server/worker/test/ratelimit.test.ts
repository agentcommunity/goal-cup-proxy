// The device-flow rate limit (src/ratelimit.ts) against the real DeviceLimiter class behind a fake
// Durable Object namespace, the IPv6 /64 key, the missing-binding refusal, and the health check.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import worker from "../src/index.ts";
import { DeviceLimiter, clientAddress, rateLimit } from "../src/ratelimit.ts";

/** A namespace that keeps one DeviceLimiter per name, as Cloudflare does, and records the names. */
function fakeNamespace() {
  const objects = new Map<string, DeviceLimiter>();
  const names: string[] = [];
  const ns = {
    idFromName: (name: string) => {
      names.push(name);
      return name;
    },
    get: (id: string) => {
      if (!objects.has(id)) objects.set(id, new DeviceLimiter());
      const obj = objects.get(id)!;
      return { fetch: (url: string, init: RequestInit) => obj.fetch(new Request(url, init)) };
    },
  };
  return { names, objects, binding: ns as unknown as DurableObjectNamespace };
}

const req = (ip: string | null) => new Request("https://x/v1/device", { method: "POST", headers: ip === null ? {} : { "cf-connecting-ip": ip } });
const limited = (e: unknown) => e instanceof ApiError && e.status === 429 && e.code === "rate_limited" && Number(e.headers["retry-after"]) >= 1 && Number(e.headers["retry-after"]) <= 60;

test("over the limit is 429 rate_limited with Retry-After, and the key holds no raw address", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, DEVICE_START_PER_MINUTE: "2" } as unknown as Env;
  await rateLimit(env, req("203.0.113.5"), "start");
  await rateLimit(env, req("203.0.113.5"), "start");
  await assert.rejects(rateLimit(env, req("203.0.113.5"), "start"), limited);
  await rateLimit(env, req("203.0.113.6"), "start"); // another address has its own count
  await rateLimit(env, req("203.0.113.5"), "token"); // and so has the other call
  assert.match(f.names[0]!, /^start:[0-9a-f]{32}$/);
  assert.ok(!f.names.some((k) => k.includes("203.0.113")));
});

test("the default limits leave a participant's normal polling far below the limit", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding } as unknown as Env;
  for (let i = 0; i < 12 * 5; i++) await rateLimit(env, req("198.51.100.1"), "token"); // five tracers polling every 5 s
  await rateLimit(env, req("198.51.100.1"), "start");
});

test("an IPv6 client is counted by its /64", async () => {
  assert.equal(clientAddress("2001:db8:aa:bb:1:2:3:4"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("2001:DB8:AA:BB::99"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("2001:db8::1"), "2001:db8:0:0::/64");
  assert.equal(clientAddress("2001:0db8:00aa:00bb:ffff:ffff:ffff:ffff"), "2001:db8:aa:bb::/64");
  assert.equal(clientAddress("::ffff:192.0.2.1"), "192.0.2.1");
  assert.equal(clientAddress("192.0.2.1"), "192.0.2.1");
  assert.equal(clientAddress(null), "unknown");

  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, DEVICE_START_PER_MINUTE: "1" } as unknown as Env;
  await rateLimit(env, req("2001:db8:aa:bb::1"), "start");
  await assert.rejects(rateLimit(env, req("2001:db8:aa:bb:ffff::2"), "start"), limited); // same /64: same count
  await rateLimit(env, req("2001:db8:aa:bc::1"), "start"); // the next /64 is another client
});

test("no limiter on the local backend means no limit", async () => {
  await rateLimit({ DB_BACKEND: "local" } as unknown as Env, req("203.0.113.5"), "start");
});

test("no limiter on the Supabase backend refuses the call with 500 misconfigured and says why in the log", async () => {
  const logged: string[] = [];
  const real = console.error;
  console.error = (...args: unknown[]) => void logged.push(args.map(String).join(" "));
  try {
    for (const env of [{}, { DB_BACKEND: "supabase" }]) {
      await assert.rejects(rateLimit(env as unknown as Env, req("203.0.113.5"), "start"), (e) => e instanceof ApiError && e.status === 500 && e.code === "misconfigured");
    }
    // Through the Worker too: a deployed Worker without the binding never runs the device flow unlimited.
    const env = { DB_BACKEND: "supabase", SUPABASE_URL: "https://example.supabase.co", SUPABASE_KEY: "k", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
    for (const path of ["/v1/device", "/v1/device/token"]) {
      const res = await worker.fetch(new Request(`https://x${path}`, { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: "{}" }), env);
      assert.equal(res.status, 500);
      assert.equal(((await res.json()) as { error: string }).error, "misconfigured");
    }
  } finally {
    console.error = real;
  }
  assert.ok(logged.some((l) => /DEVICE_LIMITER/.test(l)));
});

test("a limit that is not a positive integer is misconfigured", async () => {
  const f = fakeNamespace();
  const real = console.error;
  console.error = () => {};
  try {
    await assert.rejects(rateLimit({ DEVICE_LIMITER: f.binding, DEVICE_TOKEN_PER_MINUTE: "lots" } as unknown as Env, req("203.0.113.5"), "token"), (e) => e instanceof ApiError && e.code === "misconfigured");
  } finally {
    console.error = real;
  }
});

test("POST /v1/device answers 429 before any database call when the limit is spent", async () => {
  const f = fakeNamespace();
  const env = { DEVICE_LIMITER: f.binding, DEVICE_START_PER_MINUTE: "1", DB_BACKEND: "local", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
  const call = () => worker.fetch(new Request("https://x/v1/device", { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: "{}" }), env);
  assert.equal((await call()).status, 400); // under the limit: the body is checked (and is malformed)
  const res = await call();
  assert.equal(res.status, 429);
  assert.deepEqual(await res.json(), { error: "rate_limited", message: "Too many requests from this address; wait a minute and try again." });
});

test("GET /v1/health answers ok without configuration", async () => {
  const res = await worker.fetch(new Request("https://x/v1/health"), {} as Env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { status: "ok" });
});
