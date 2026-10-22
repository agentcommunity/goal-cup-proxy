// Rate limits on the device flow (POST /v1/device and /v1/device/token), the only calls that
// carry no credential. Each limit is one Durable Object per (limit, key), counting the calls of the
// current minute. A key is an HMAC-SHA-256 under the RATE_LIMIT_KEY secret, so neither an object's
// name nor a log line can be reversed to an address (IPv4 has too few addresses for a plain hash).
//
//   start  POST /v1/device, per client address (an IPv6 client by its /64)
//   poll   POST /v1/device/token, per device code: one goalcup polls one code every 5 seconds
//          (12 a minute), and people behind one venue or campus NAT each poll their own code
//   token  POST /v1/device/token, per client address: a high cap against abuse only
//
// Why not Cloudflare's Rate Limiting binding (`[[ratelimits]]`), which this file used until
// 2026-10-15: it was attached and answered, but its counters stayed local to the machine that
// served the call. A burst spread over a location's machines never tripped it (172 polls in 26 s at
// one location against a 120/min limit, and 19 starts against 5/min: all allowed). A Durable Object
// is one counter per key, so the limit is the limit.
//
// The limits are wrangler.toml vars (DEVICE_START_PER_MINUTE, DEVICE_POLL_PER_MINUTE,
// DEVICE_TOKEN_PER_MINUTE). No limiter bound: a local or test run has no limit; a deployment on the
// Supabase backend refuses the call with 500 misconfigured instead of running unlimited, and so
// does a bound limiter without RATE_LIMIT_KEY (DB_BACKEND=local falls back to a fixed local key).

import type { Env } from "./env.ts";
import { ApiError } from "./errors.ts";
import { toHex } from "./util.ts";

export type LimitScope = "start" | "poll" | "token";

const WINDOW_MS = 60_000;
const DEFAULT_PER_MINUTE: Record<LimitScope, number> = { start: 120, poll: 30, token: 1200 };
const VAR: Record<LimitScope, "DEVICE_START_PER_MINUTE" | "DEVICE_POLL_PER_MINUTE" | "DEVICE_TOKEN_PER_MINUTE"> = {
  start: "DEVICE_START_PER_MINUTE",
  poll: "DEVICE_POLL_PER_MINUTE",
  token: "DEVICE_TOKEN_PER_MINUTE",
};
const LOCAL_RATE_LIMIT_KEY = "local-only rate limit key, never a deployment's";
const MISCONFIGURED = "The rate limiter is not configured; the Worker's log says how.";

/** The per-address limits: `start` for POST /v1/device, `token` (the abuse cap) for POST /v1/device/token. */
export async function limitAddress(env: Env, request: Request, scope: "start" | "token"): Promise<void> {
  await limit(env, scope, clientAddress(request.headers.get("cf-connecting-ip")));
}

/** The per-code limit on POST /v1/device/token, once the body has named the device code. */
export async function limitDeviceCode(env: Env, deviceCode: string): Promise<void> {
  await limit(env, "poll", deviceCode);
}

async function limit(env: Env, scope: LimitScope, value: string): Promise<void> {
  const limiter = env.DEVICE_LIMITER;
  if (limiter === undefined) {
    if ((env.DB_BACKEND ?? "supabase") === "local") return;
    console.error("misconfigured: the DEVICE_LIMITER Durable Object binding is missing; refusing the device flow rather than running it unlimited");
    throw new ApiError(500, "misconfigured", MISCONFIGURED);
  }
  const perMinute = limitOf(env, scope);
  // `wrangler dev` and the tests (DB_BACKEND=local) may run without the secret; a deployment may not.
  const secret = env.RATE_LIMIT_KEY ?? (env.DB_BACKEND === "local" ? LOCAL_RATE_LIMIT_KEY : undefined);
  const key = `${scope}:${(await keyedHash(secret, `${scope}\0${value}`)).slice(0, 32)}`;
  const res = await limiter.get(limiter.idFromName(key)).fetch("https://limiter/hit", { method: "POST", body: JSON.stringify({ limit: perMinute }) });
  if (!res.ok) throw new Error(`device limiter answered ${res.status}`);
  const { allowed, retry_after } = (await res.json()) as { allowed: boolean; retry_after: number };
  if (!allowed) {
    const message = scope === "poll" ? "This sign-in is polled too often; wait a minute and try again." : "Too many requests from this address; wait a minute and try again.";
    throw new ApiError(429, "rate_limited", message, { "retry-after": String(retry_after) });
  }
}

function limitOf(env: Env, scope: LimitScope): number {
  const raw = env[VAR[scope]];
  if (raw === undefined || raw === "") return DEFAULT_PER_MINUTE[scope];
  const n = Number(raw);
  if (!Number.isInteger(n) || n <= 0) {
    console.error(`misconfigured: ${VAR[scope]} must be a positive integer`);
    throw new ApiError(500, "misconfigured", MISCONFIGURED);
  }
  return n;
}

let cachedKey: { secret: string; key: CryptoKey } | undefined;

/** HMAC-SHA-256 of `data` under the RATE_LIMIT_KEY secret (at least 32 characters), as hex. */
async function keyedHash(secret: string | undefined, data: string): Promise<string> {
  if (secret === undefined || secret.length < 32) {
    console.error("misconfigured: RATE_LIMIT_KEY must be a secret of at least 32 characters");
    throw new ApiError(500, "misconfigured", MISCONFIGURED);
  }
  if (cachedKey?.secret !== secret) {
    const key = await crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
    cachedKey = { secret, key };
  }
  return toHex(new Uint8Array(await crypto.subtle.sign("HMAC", cachedKey.key, new TextEncoder().encode(data))));
}

/**
 * The address a limit counts: IPv4 as it is, IPv6 as its /64 (an IPv4-mapped IPv6 address as the
 * IPv4 address). Anything unparsable counts as itself, lowercased; no address at all is "unknown".
 */
export function clientAddress(ip: string | null): string {
  if (ip === null || ip === "") return "unknown";
  const addr = ip.trim().toLowerCase();
  if (!addr.includes(":")) return addr;
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/.exec(addr);
  if (mapped !== null) return mapped[1]!;
  const halves = addr.split("::");
  if (halves.length > 2) return addr;
  const groups = (s: string | undefined): string[] => (s === undefined || s === "" ? [] : s.split(":").flatMap((g) => (g.includes(".") ? ["0", "0"] : [g])));
  const head = groups(halves[0]);
  const tail = groups(halves[1]);
  const fill = halves.length === 2 ? 8 - head.length - tail.length : 0;
  if (fill < 0 || (halves.length === 1 && head.length !== 8)) return addr;
  const all = [...head, ...Array<string>(fill).fill("0"), ...tail];
  if (!all.every((g) => /^[0-9a-f]{1,4}$/.test(g))) return addr;
  return `${all.slice(0, 4).map((g) => Number.parseInt(g, 16).toString(16)).join(":")}::/64`;
}

/**
 * The Durable Object behind DEVICE_LIMITER: one instance per (call, client address), counting the
 * calls of a fixed one-minute window in memory. Memory is enough: an instance that goes idle and is
 * evicted has nothing worth remembering, and while a burst lasts the instance stays.
 */
export class DeviceLimiter {
  private windowStart = 0;
  private count = 0;

  constructor(_state?: unknown, _env?: unknown) {}

  async fetch(request: Request): Promise<Response> {
    const { limit } = (await request.json()) as { limit: number };
    // No await from here on: the read and the update of the counter are one step.
    const now = Date.now();
    if (now - this.windowStart >= WINDOW_MS) {
      this.windowStart = now;
      this.count = 0;
    }
    this.count += 1;
    const retry_after = Math.max(1, Math.ceil((this.windowStart + WINDOW_MS - now) / 1000));
    return Response.json({ allowed: this.count <= limit, retry_after });
  }
}
