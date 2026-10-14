// Goal Cup upload Worker: routing, maintenance switch, authentication, error mapping.
// Contract: docs/UPLOAD-API.md. No framework; six routes.

import { authenticate } from "./auth.ts";
import { putChunk } from "./chunks.ts";
import { type Ctx, makeCtx } from "./context.ts";
import type { Env } from "./env.ts";
import { ApiError, errorResponse, json } from "./errors.ts";
import { abandonRun, getPosition, issueReceipt, openRun, sealRun } from "./runs.ts";
import { RUN_ID } from "./util.ts";

type Handler = (ctx: Ctx, member: string, request: Request) => Promise<Response>;
interface Route {
  method: string;
  handler: Handler;
}

/** Match the path; returns the allowed handlers by method, or null for an unknown path. */
function route(path: string): Route[] | null {
  const parts = path.split("/"); // "/v1/runs/r…/seal" -> ["", "v1", "runs", "r…", "seal"]
  if (parts[0] !== "" || parts[1] !== "v1" || parts[2] !== "runs") return null;
  if (parts.length === 3) return [{ method: "POST", handler: (c, m, r) => openRun(c, m, r) }];

  const runId = parts[3]!;
  if (!RUN_ID.test(runId)) return null;
  if (parts.length === 4) return [{ method: "GET", handler: (c, m) => getPosition(c, m, runId) }];
  if (parts.length === 5) {
    switch (parts[4]) {
      case "receipt":
        return [{ method: "POST", handler: (c, m, r) => issueReceipt(c, m, runId, r) }];
      case "abandon":
        return [{ method: "POST", handler: (c, m, r) => abandonRun(c, m, runId, r) }];
      case "seal":
        return [{ method: "POST", handler: (c, m, r) => sealRun(c, m, runId, r) }];
    }
  }
  if (parts.length === 6 && parts[4] === "chunks") {
    const offset = parts[5]!;
    return [{ method: "PUT", handler: (c, m, r) => putChunk(c, m, runId, offset, r) }];
  }
  return null;
}

async function handle(request: Request, env: Env): Promise<Response> {
  const routes = route(new URL(request.url).pathname);
  if (routes === null) throw new ApiError(404, "not_found", "No such endpoint.");
  const match = routes.find((r) => r.method === request.method);
  if (match === undefined) {
    throw new ApiError(405, "method_not_allowed", "That method is not allowed here.", {
      allow: routes.map((r) => r.method).join(", "),
    });
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
  const member = await authenticate(ctx, request);
  return match.handler(ctx, member, request);
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
