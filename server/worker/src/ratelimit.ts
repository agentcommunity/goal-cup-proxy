// Rate limits on the device flow (POST /v1/device and /v1/device/token), the only calls that
// carry no credential. One Durable Object per (call, client address) counts the calls of the
// current minute; the address is hashed before it becomes the object's name (no raw IP in a key),
// and an IPv6 client is counted by its /64, the block one subscriber usually holds.
//
// Why not Cloudflare's Rate Limiting binding (`[[ratelimits]]`), which this file used until
// 2026-10-15: it was attached and answered, but its counters stayed local to the machine that
// served the call. A burst spread over a location's machines never tripped it (172 polls in 26 s at
// one location against a 120/min limit, and 19 starts against 5/min: all allowed). A Durable Object
// is one counter per key, so the limit is the limit.
//
// The limits (wrangler.toml DEVICE_START_PER_MINUTE, DEVICE_TOKEN_PER_MINUTE) sit far above one
// participant's normal use: a start is one call, and polling is one call every 5 seconds (12 a
// minute) for at most 15 minutes. No limiter bound: a local or test run has no limit; a deployment
// on the Supabase backend refuses the call with 500 misconfigured instead of running unlimited.

import type { Env } from "./env.ts";
import { ApiError } from "./errors.ts";
import { sha256Hex } from "./util.ts";

export type LimitScope = "start" | "token";

const WINDOW_MS = 60_000;
const DEFAULT_PER_MINUTE: Record<LimitScope, number> = { start: 30, token: 300 };

export async function rateLimit(env: Env, request: Request, scope: LimitScope): Promise<void> {
  const limiter = env.DEVICE_LIMITER;
  if (limiter === undefined) {
    if ((env.DB_BACKEND ?? "supabase") === "local") return;
    console.error("misconfigured: the DEVICE_LIMITER Durable Object binding is missing; refusing the device flow rather than running it unlimited");
    throw new ApiError(500, "misconfigured", "The rate limiter is not configured; the Worker's log says how.");
  }
  const limit = perMinute(env, scope);
  const key = `${scope}:${(await sha256Hex(clientAddress(request.headers.get("cf-connecting-ip")))).slice(0, 32)}`;
  const res = await limiter.get(limiter.idFromName(key)).fetch("https://limiter/hit", { method: "POST", body: JSON.stringify({ limit }) });
  if (!res.ok) throw new Error(`device limiter answered ${res.status}`);
  const { allowed, retry_after } = (await res.json()) as { allowed: boolean; retry_after: number };
  if (!allowed) {
    throw new ApiError(429, "rate_limited", "Too many requests from this address; wait a minute and try again.", { "retry-after": String(retry_after) });
  }
}

function perMinute(env: Env, scope: LimitScope): number {
  const raw = scope === "start" ? env.DEVICE_START_PER_MINUTE : env.DEVICE_TOKEN_PER_MINUTE;
  if (raw === undefined || raw === "") return DEFAULT_PER_MINUTE[scope];
  const n = Number(raw);
  if (!Number.isInteger(n) || n <= 0) {
    console.error(`misconfigured: DEVICE_${scope.toUpperCase()}_PER_MINUTE must be a positive integer`);
    throw new ApiError(500, "misconfigured", "The rate limiter is not configured; the Worker's log says how.");
  }
  return n;
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
