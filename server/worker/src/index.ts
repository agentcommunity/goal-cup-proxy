// Goal Cup upload Worker: routing, maintenance switch, authentication, error mapping, and the
// 5-minute cron (notices.ts). Contract: docs/UPLOAD-API.md. No framework; eleven routes and a health check.

import { type Auth, authenticate } from "./auth.ts";
import { putChunk } from "./chunks.ts";
import { type Ctx, makeCtx } from "./context.ts";
import { approveDevice, createDevice, pollDevice } from "./device.ts";
import type { Env } from "./env.ts";
import { ApiError, errorResponse, json, notFound } from "./errors.ts";
import { runTick } from "./notices.ts";
import { abandonRun, continueRun, getPosition, heartbeat, issueReceipt, openRun, sealRun } from "./runs.ts";
import { RUN_ID } from "./util.ts";

type Handler = (ctx: Ctx, auth: Auth, request: Request) => Promise<Response>;
/** The device flow is how a credential comes to exist, so its calls carry none. */
type PublicHandler = (ctx: Ctx, request: Request) => Promise<Response>;
type Route = { method: string } & (
  | { public: true; handler: PublicHandler }
  /** `runId` is the run the path names (null for a path that names none): only a credential bound to it may pass. */
  | { public: false; runId: string | null; handler: Handler }
);

/** Match the method and path; null for an unknown path. A known path with the wrong method is the caller's 405. */
function route(path: string): Route | null {
  const parts = path.split("/"); // "/v1/runs/r…/seal" -> ["", "v1", "runs", "r…", "seal"]
  if (parts[0] !== "" || parts[1] !== "v1") return null;
  if (parts[2] === "device") {
    const open = (handler: PublicHandler): Route => ({ method: "POST", public: true, handler });
    if (parts.length === 3) return open(createDevice);
    if (parts.length === 4 && parts[3] === "token") return open(pollDevice);
    if (parts.length === 4 && parts[3] === "approve") return open(approveDevice);
    return null;
  }
  if (parts[2] !== "runs") return null;
  const authed = (method: string, runId: string | null, handler: Handler): Route => ({ method, public: false, runId, handler });
  if (parts.length === 3) return authed("POST", null, openRun);

  const runId = parts[3]!;
  if (!RUN_ID.test(runId)) return null;
  if (parts.length === 4) return authed("GET", runId, (c, a, r) => getPosition(c, a.member, runId, r));
  if (parts.length === 5) {
    switch (parts[4]) {
      case "receipt":
        return authed("POST", runId, (c, a, r) => issueReceipt(c, a.member, runId, r));
      case "abandon":
        return authed("POST", runId, (c, a, r) => abandonRun(c, a.member, runId, r));
      case "seal":
        return authed("POST", runId, (c, a, r) => sealRun(c, a.member, runId, r));
      case "heartbeat":
        return authed("POST", runId, (c, a, r) => heartbeat(c, a.member, runId, r));
      case "continue":
        return authed("POST", runId, (c, a, r) => continueRun(c, a, runId, r));
    }
  }
  if (parts.length === 6 && parts[4] === "chunks") {
    const offset = parts[5]!;
    return authed("PUT", runId, (c, a, r) => putChunk(c, a.member, runId, offset, r));
  }
  return null;
}

async function handle(request: Request, env: Env): Promise<Response> {
  const path = new URL(request.url).pathname;
  // Liveness only: no database, no configuration, nothing about runs.
  if (path === "/v1/health" && request.method === "GET") return json(200, { status: "ok" });
  const matched = route(path);
  if (matched === null) throw new ApiError(404, "not_found", "No such endpoint.");
  if (request.method !== matched.method) {
    throw new ApiError(405, "method_not_allowed", "That method is not allowed here.", { allow: matched.method });
  }

  // The maintenance switch comes before everything that costs a database call.
  if (env.MAINTENANCE === "on") {
    const secs = Number(env.MAINTENANCE_RETRY_AFTER);
    const retryAfter = Number.isInteger(secs) && secs > 0 ? secs : 300;
    throw new ApiError(503, "maintenance", "The server asks every tracer to wait; try again later.", {
      "retry-after": String(retryAfter),
    });
  }

  const ctx = await makeCtx(env);
  if (matched.public) return matched.handler(ctx, request);

  const auth = await authenticate(ctx, request);
  // A credential sees the run it opened and nothing else; before it has opened one it sees no run.
  if (matched.runId !== null && auth.run_id !== matched.runId) throw notFound("No such run for this credential.");
  return matched.handler(ctx, auth, request);
}

// The Durable Object class behind DEVICE_LIMITER (wrangler.toml); wrangler finds it by this export.
export { DeviceLimiter } from "./ratelimit.ts";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    try {
      return await handle(request, env);
    } catch (e) {
      if (e instanceof ApiError) return errorResponse(e);
      console.error(`unhandled: ${e instanceof Error ? (e.stack ?? e.message) : String(e)}`);
      return json(500, { error: "internal", message: "Something went wrong on the server; try again." });
    }
  },

  /** The cron (wrangler.toml `[triggers]`): close runs, record the deadline commit, queue the run emails. */
  async scheduled(_controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(runTick(env));
  },
} satisfies ExportedHandler<Env>;
