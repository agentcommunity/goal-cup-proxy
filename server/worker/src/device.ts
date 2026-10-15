// The device flow (docs/UPLOAD-API.md, "Sign in and the run credential"). The tracer asks for a
// code, the member approves it on the website, and the tracer picks up a run credential once.
//
//   POST /v1/device          no authentication: start a request
//   POST /v1/device/token    no authentication: poll it with the device code (in the body, never a URL)
//   POST /v1/device/approve  tests and staging only: approve a request as any member
//
// Only the sha256 of the device code, the user code and the credential is stored.

import type { Ctx } from "./context.ts";
import type { DevicePurpose, RunMode } from "./db.ts";
import { ApiError, closed, json, malformed, notFound, unauthorized } from "./errors.ts";
import { rateLimit } from "./ratelimit.ts";
import { readJson } from "./runs.ts";
import { addSeconds, constantTimeEqual, formatTime, hexField, sha256Hex, strField, toHex } from "./util.ts";

/** The letters of a user code: no vowels, so no word, and nothing that looks like another character. */
export const USER_CODE_ALPHABET = "BCDFGHJKLMNPQRSTVWXZ";
const USER_CODE_LETTERS = 8;
const DEVICE_EXPIRES_IN = 900; // seconds a request lives
const DEVICE_INTERVAL = 5; // seconds between polls
const BODY_MAX = 4096;

export const newDeviceCode = (): string => toHex(crypto.getRandomValues(new Uint8Array(32)));

/** 8 random letters shown as `XXXX-XXXX`. Bytes of 240 and up are skipped so every letter is equally likely. */
export function newUserCode(): string {
  const limit = 256 - (256 % USER_CODE_ALPHABET.length);
  let letters = "";
  while (letters.length < USER_CODE_LETTERS) {
    for (const b of crypto.getRandomValues(new Uint8Array(16))) {
      if (b < limit && letters.length < USER_CODE_LETTERS) letters += USER_CODE_ALPHABET[b % USER_CODE_ALPHABET.length];
    }
  }
  return `${letters.slice(0, 4)}-${letters.slice(4)}`;
}

/** What a member typed, as the 8 stored letters; null if it cannot be a user code. Case and the dash do not matter. */
export function normalizeUserCode(input: string): string | null {
  const m = /^([A-Za-z]{4})-?([A-Za-z]{4})$/.exec(input);
  if (m === null) return null;
  const code = (m[1]! + m[2]!).toUpperCase();
  return [...code].every((c) => USER_CODE_ALPHABET.includes(c)) ? code : null;
}

// ---- POST /v1/device ----------------------------------------------------------------------

export async function createDevice(ctx: Ctx, request: Request): Promise<Response> {
  await rateLimit(ctx.env, request, "start");
  const b = await readJson(request, BODY_MAX);
  const purpose = enumField<DevicePurpose>(b, "purpose", ["login", "run"]);
  const mode = enumField<RunMode>(b, "mode", ["competition", "practice", "test"]);
  const connector_id = strField(b, "connector_id", 128);
  const tracer_version = strField(b, "tracer_version", 64);

  let uri: URL;
  try {
    uri = new URL(ctx.env.APPROVAL_URL ?? "");
  } catch {
    throw new ApiError(500, "misconfigured", "APPROVAL_URL is not set to the approval page's address.");
  }

  const device_code = newDeviceCode();
  const user_code = newUserCode();
  const now = ctx.now();
  await ctx.db.createDevice({
    device_sha256: await sha256Hex(device_code),
    user_sha256: await sha256Hex(user_code.replace("-", "")),
    purpose,
    mode,
    connector_id,
    tracer_version,
    created_at: formatTime(now),
    expires_at: formatTime(addSeconds(now, DEVICE_EXPIRES_IN)),
  });

  const verification_uri = uri.toString();
  uri.searchParams.set("code", user_code);
  return json(200, {
    device_code,
    user_code,
    verification_uri,
    verification_uri_complete: uri.toString(),
    interval: DEVICE_INTERVAL,
    expires_in: DEVICE_EXPIRES_IN,
  });
}

function enumField<T extends string>(o: Record<string, unknown>, name: string, allowed: readonly T[]): T {
  const v = o[name];
  if (typeof v !== "string" || !(allowed as readonly string[]).includes(v)) throw malformed(`${name} must be ${allowed.map((a) => `"${a}"`).join(" or ")}.`);
  return v as T;
}

// ---- POST /v1/device/token ----------------------------------------------------------------

export async function pollDevice(ctx: Ctx, request: Request): Promise<Response> {
  await rateLimit(ctx.env, request, "token");
  const b = await readJson(request, BODY_MAX);
  const deviceSha = await sha256Hex(hexField(b, "device_code"));
  const poll = await ctx.db.pollDevice(deviceSha, formatTime(ctx.now()));
  switch (poll.kind) {
    case "pending":
      return json(202, { status: "pending" });
    case "approved":
      if (poll.purpose === "login") return json(200, { status: "approved", purpose: "login", account: { handle: poll.handle } });
      return json(200, { status: "approved", purpose: "run", credential: poll.credential_plain, expires_at: poll.expires_at });
    case "denied":
      throw new ApiError(403, "forbidden", "This sign-in was denied on the website.");
    case "expired":
      throw closed("This sign-in request has expired or was already picked up. Start again.");
  }
}

// ---- POST /v1/device/approve (tests and staging) ------------------------------------------

export async function approveDevice(ctx: Ctx, request: Request): Promise<Response> {
  // Switched on by the key, and only ever by it: no key, no route. (A key outside local or
  // staging never gets here: readConfig refuses every call first.)
  const key = ctx.env.TEST_APPROVAL_KEY;
  if (key === undefined || key === "") throw notFound("No such endpoint.");
  const given = /^Bearer (.{1,512})$/.exec(request.headers.get("authorization") ?? "")?.[1];
  if (given === undefined || !(await constantTimeEqual(given, key))) throw unauthorized("The approval key is missing or wrong.");

  const b = await readJson(request, BODY_MAX);
  const code = normalizeUserCode(strField(b, "user_code", 16));
  if (code === null) throw malformed("user_code is not a user code.");
  const member = strField(b, "member_id", 128);
  if (typeof b.approve !== "boolean") throw malformed("approve must be true or false.");

  switch (await ctx.db.approveDevice(code, member, b.approve, formatTime(ctx.now()))) {
    case "ok":
      return json(200, { status: b.approve ? "approved" : "denied" });
    case "not_found":
      throw notFound("No such user code.");
    case "expired":
      throw closed("That sign-in request has expired.");
    case "already_decided":
      throw new ApiError(409, "conflict", "That sign-in request was already decided.");
    case "forbidden":
      throw new ApiError(403, "forbidden", "The test approval works only for staging requests and staging test members.");
  }
}
