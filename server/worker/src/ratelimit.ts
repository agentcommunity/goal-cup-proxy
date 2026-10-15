// Rate limits on the device flow (POST /v1/device and /v1/device/token), the only calls that
// carry no credential. Cloudflare's Rate Limiting binding, per client address and per
// Cloudflare location; the address is hashed before it becomes a key (no raw IP in a key).
// Unbound (local runs, tests, staging) means no limit.
//
// The limits (wrangler.toml [[ratelimits]]) sit far above one participant's normal use: a start
// is one call, and polling is one call every 5 seconds (12 a minute) for at most 15 minutes.

import { ApiError } from "./errors.ts";
import { sha256Hex } from "./util.ts";

export async function rateLimit(binding: RateLimit | undefined, request: Request, scope: string): Promise<void> {
  if (binding === undefined) return;
  const ip = request.headers.get("cf-connecting-ip") ?? "unknown";
  const key = `${scope}:${(await sha256Hex(ip)).slice(0, 32)}`;
  const { success } = await binding.limit({ key });
  if (!success) {
    throw new ApiError(429, "rate_limited", "Too many requests from this address; wait a minute and try again.", { "retry-after": "60" });
  }
}
