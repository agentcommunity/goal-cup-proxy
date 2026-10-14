// Bearer-credential authentication. The server never stores a credential, only its sha256.
// A run credential (`gcr_` + 64 hex) comes from the device flow (device.ts). It stands for one
// member and, once it has opened a run, for that run only.

import type { Ctx } from "./context.ts";
import type { RunMode } from "./db.ts";
import { unauthorized } from "./errors.ts";
import { formatTime, sha256Hex } from "./util.ts";

const BEARER = /^Bearer (gcr_[0-9a-f]{64})$/;

export interface Auth {
  member: string;
  /** sha256 of the credential, to bind it to the run it opens. */
  credential: string;
  /** The one run this credential may touch; null until it has opened one. */
  run_id: string | null;
  mode: RunMode;
}

/** Who the request's credential is, or throw 401. */
export async function authenticate(ctx: Ctx, request: Request): Promise<Auth> {
  const m = BEARER.exec(request.headers.get("authorization") ?? "");
  if (m === null) throw unauthorized("Send your run credential as 'Authorization: Bearer <credential>'.");
  const hash = await sha256Hex(m[1]!);
  const cred = await ctx.db.credential(hash, formatTime(ctx.now()));
  if (cred === null) throw unauthorized("That credential is not known or has expired. Run `goalcup login` again.");
  return { member: cred.member_id, credential: hash, run_id: cred.run_id, mode: cred.mode };
}
