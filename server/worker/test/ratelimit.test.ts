// The device-flow rate limit (src/ratelimit.ts) against a fake binding, and the health check.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { Env } from "../src/env.ts";
import { ApiError } from "../src/errors.ts";
import worker from "../src/index.ts";
import { rateLimit } from "../src/ratelimit.ts";

function fakeLimit(allowed: number) {
  const keys: string[] = [];
  return {
    keys,
    binding: { limit: async ({ key }: { key: string }) => ({ success: keys.push(key) <= allowed }) } as unknown as RateLimit,
  };
}

const req = (ip: string | null) => new Request("https://x/v1/device", { method: "POST", headers: ip === null ? {} : { "cf-connecting-ip": ip } });

test("no binding means no limit", async () => {
  await rateLimit(undefined, req("203.0.113.5"), "start");
});

test("over the limit is 429 rate_limited with Retry-After, and the key holds no raw address", async () => {
  const f = fakeLimit(1);
  await rateLimit(f.binding, req("203.0.113.5"), "start");
  await assert.rejects(rateLimit(f.binding, req("203.0.113.5"), "start"), (e) => e instanceof ApiError && e.status === 429 && e.code === "rate_limited" && e.headers["retry-after"] === "60");
  assert.match(f.keys[0]!, /^start:[0-9a-f]{32}$/);
  assert.ok(!f.keys.some((k) => k.includes("203.0.113")));
});

test("POST /v1/device answers 429 before any database call when the limit is spent", async () => {
  const f = fakeLimit(0);
  const env = { RL_DEVICE_START: f.binding, DB_BACKEND: "local", EVENT_ID: "cup", PRACTICE_EVENT_ID: "practice", RECEIPT_SIGNING_KEY: "07".repeat(32), TRACES: {} } as unknown as Env;
  const res = await worker.fetch(new Request("https://x/v1/device", { method: "POST", headers: { "cf-connecting-ip": "198.51.100.7", "content-type": "application/json" }, body: "{}" }), env);
  assert.equal(res.status, 429);
  assert.deepEqual(await res.json(), { error: "rate_limited", message: "Too many requests from this address; wait a minute and try again." });
});

test("GET /v1/health answers ok without configuration", async () => {
  const res = await worker.fetch(new Request("https://x/v1/health"), {} as Env);
  assert.equal(res.status, 200);
  assert.deepEqual(await res.json(), { status: "ok" });
});
