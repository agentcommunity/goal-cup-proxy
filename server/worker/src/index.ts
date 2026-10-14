// Goal Cup upload Worker: routing, maintenance switch, authentication, error mapping.
// Contract: docs/UPLOAD-API.md. No framework; ten routes.

import { type Auth, authenticate } from "./auth.ts";
import { putChunk } from "./chunks.ts";
import { type Ctx, makeCtx } from "./context.ts";
import { approveDevice, createDevice, pollDevice } from "./device.ts";
import type { Env } from "./env.ts";
import { ApiError, errorResponse, json, notFound } from "./errors.ts";
import { abandonRun, getPosition, heartbeat, issueReceipt, openRun, sealRun } from "./runs.ts";
import { RUN_ID } from "./util.ts";

type Handler = (ctx: Ctx, auth: Auth, request: Request) => Promise<Response>;
interface Route {
  method: string;
  handler: Handler;
}
/** The device flow is how a credential comes to exist, so its calls carry none. */
interface PublicRoute {
  method: string;
  handler: (ctx: Ctx, request: Request) => Promise<Response>;
}
type Matched =
  | { public: true; routes: PublicRoute[] }
  /** `runId` is the run the path names: only a credential bound to it may pass. */
  | { public: false; routes: Route[]; runId: string | null };

/** Match the path; returns the allowed handlers by method, or null for an unknown path. */
function route(path: string): Matched | null {
  const parts = path.split("/"); // "/v1/runs/r…/seal" -> ["", "v1", "runs", "r…", "seal"]
  if (parts[0] !== "" || parts[1] !== "v1") return null;
  if (parts[2] === "device") {
    if (parts.length === 3) return { public: true, routes: [{ method: "POST", handler: createDevice }] };
    if (parts.length === 4 && parts[3] === "token") return { public: true, routes: [{ method: "POST", handler: pollDevice }] };
    if (parts.length === 4 && parts[3] === "approve") return { public: true, routes: [{ method: "POST", handler: approveDevice }] };
    return null;
  }
  if (parts[2] !== "runs") return null;
  const runs = (routes: Route[], runId: string | null): Matched => ({ public: false, routes, runId });
  if (parts.length === 3) return runs([{ method: "POST", handler: (c, a, r) => openRun(c, a, r) }], null);

  const runId = parts[3]!;
  if (!RUN_ID.test(runId)) return null;
  if (parts.length === 4) return runs([{ method: "GET", handler: (c, a) => getPosition(c, a.member, runId) }], runId);
  if (parts.length === 5) {
    switch (parts[4]) {
      case "receipt":
        return runs([{ method: "POST", handler: (c, a, r) => issueReceipt(c, a.member, runId, r) }], runId);
      case "abandon":
        return runs([{ method: "POST", handler: (c, a, r) => abandonRun(c, a.member, runId, r) }], runId);
      case "seal":
        return runs([{ method: "POST", handler: (c, a, r) => sealRun(c, a.member, runId, r) }], runId);
      case "heartbeat":
        return runs([{ method: "POST", handler: (c, a, r) => heartbeat(c, a.member, runId, r) }], runId);
    }
  }
  if (parts.length === 6 && parts[4] === "chunks") {
    const offset = parts[5]!;
    return runs([{ method: "PUT", handler: (c, a, r) => putChunk(c, a.member, runId, offset, r) }], runId);
  }
  return null;
}

async function handle(request: Request, env: Env): Promise<Response> {
  const matched = route(new URL(request.url).pathname);
  if (matched === null) throw new ApiError(404, "not_found", "No such endpoint.");
  const methods = matched.routes.map((r) => r.method);
  if (!methods.includes(request.method)) {
    throw new ApiError(405, "method_not_allowed", "That method is not allowed here.", { allow: methods.join(", ") });
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
  if (matched.public) return matched.routes.find((r) => r.method === request.method)!.handler(ctx, request);

  const auth = await authenticate(ctx, request);
  // A credential sees the run it opened and nothing else; before it has opened one it sees no run.
  if (matched.runId !== null && auth.run_id !== matched.runId) throw notFound("No such run for this credential.");
  return matched.routes.find((r) => r.method === request.method)!.handler(ctx, auth, request);
}

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
} satisfies ExportedHandler<Env>;
