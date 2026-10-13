// Bearer-token authentication. The server never stores a token, only its sha256.

import type { Ctx } from "./context.ts";
import { unauthorized } from "./errors.ts";
import { sha256Hex } from "./util.ts";

const BEARER = /^Bearer ([A-Za-z0-9._~+/=-]{1,512})$/;

/** Returns the member id for the request's token, or throws 401. */
export async function authenticate(ctx: Ctx, request: Request): Promise<string> {
  const m = BEARER.exec(request.headers.get("authorization") ?? "");
  if (m === null) throw unauthorized("Send your tracer token as 'Authorization: Bearer <token>'.");
  const member = await ctx.db.memberForToken(await sha256Hex(m[1]!));
  if (member === null) throw unauthorized("That token is not known. Run `goalcup login` again.");
  return member;
}
