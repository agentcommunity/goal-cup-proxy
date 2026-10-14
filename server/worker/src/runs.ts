// Handlers for the run-level calls: open, receipt, abandon, position, seal.
// The chunk upload is in chunks.ts. Contract: docs/UPLOAD-API.md.

import type { Auth } from "./auth.ts";
import type { Ctx } from "./context.ts";
import { type RunRow, boundCredentialExpiry, isAbandonable, isPastDeadline } from "./db.ts";
import { ApiError, closed, conflict, json, malformed, notFound, tooLarge } from "./errors.ts";
import {
  REPO_URL,
  addSeconds,
  asObject,
  formatTime,
  hexField,
  isEntryId,
  newId,
  readBodyCapped,
  sha256Hex,
  strField,
} from "./util.ts";

const JSON_BODY_MAX = 64 * 1024;
const SEAL_DOC_MAX = 16 * 1024 * 1024; // each document, bytes
const SEAL_BODY_MAX = 2 * SEAL_DOC_MAX + JSON_BODY_MAX; // two documents plus the JSON around them

/**
 * Read a JSON object body of at most `max` bytes. The raw bytes and the decoded text live only
 * inside this function, so the caller holds just the parsed object.
 */
export async function readJson(request: Request, max: number): Promise<Record<string, unknown>> {
  // A declared length over the cap is refused before a single byte is read.
  const declared = request.headers.get("content-length");
  if (declared !== null && Number(declared) > max) throw tooLarge("The request body is too large.");
  const bytes = await readBodyCapped(request, max);
  if (bytes === null) throw tooLarge("The request body is too large.");
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(bytes));
  } catch {
    throw malformed("The body is not valid JSON.");
  }
  return asObject(parsed, "The body");
}

/**
 * Take one document string out of the parsed body as UTF-8 bytes, and remove the string from the
 * body so it can be freed: a seal carries two documents of up to 16 MiB, and the Worker has 128 MB.
 */
function takeDocument(body: Record<string, unknown>, name: string): Uint8Array {
  const text = body[name];
  if (typeof text !== "string" || text === "") throw malformed(`${name} must be a non-empty string.`);
  if (!text.isWellFormed()) throw malformed(`${name} is not well-formed Unicode.`);
  delete body[name];
  const bytes = new TextEncoder().encode(text);
  if (bytes.length > SEAL_DOC_MAX) throw tooLarge(`${name} is over 16 MiB.`);
  return bytes;
}

/**
 * The caller's own run, or 404. A run that belongs to someone else is indistinguishable from none.
 * A run that was never sealed 24 hours after its deadline is abandoned here, the first time anyone
 * looks at it, so every call sees it as abandoned (410) and its account's open slot is free.
 */
export async function requireRun(ctx: Ctx, member: string, runId: string): Promise<RunRow> {
  const run = await ctx.db.getRun(member, runId);
  if (run === null) throw notFound("No such run for this credential.");
  if (run.status === "open" && isAbandonable(run.deadline_at, ctx.now(), ctx.cfg.abandonAfterSeconds)) {
    if (await ctx.db.expireRun(member, runId, formatTime(ctx.now()))) return { ...run, status: "abandoned" };
    return (await ctx.db.getRun(member, runId)) ?? run;
  }
  return run;
}

/** 410 unless the run is open (not sealed, not abandoned). */
export function requireOpen(run: RunRow): void {
  if (run.status !== "open") throw closed(`This run is ${run.status}.`);
}

/** `stop`: the server's clock is at or past `deadline_at`; the run is over. */
export const isStopped = (ctx: Ctx, run: RunRow): boolean => isPastDeadline(run.deadline_at, ctx.now());

// ---- POST /v1/runs ------------------------------------------------------------------------

export async function openRun(ctx: Ctx, auth: Auth, request: Request): Promise<Response> {
  const b = await readJson(request, JSON_BODY_MAX);
  const connector_id = strField(b, "connector_id", 128);
  const manifest_sha256 = hexField(b, "manifest_sha256");
  const connector_dir_sha256 = hexField(b, "connector_dir_sha256");
  const workspace = strField(b, "workspace", 4096);
  const session_id = strField(b, "session_id", 128);
  if (!/^[A-Za-z0-9._:-]+$/.test(session_id)) throw malformed("session_id has characters a session id never has.");
  const harness_version = strField(b, "harness_version", 64);
  const tracer_version = strField(b, "tracer_version", 64);
  const repo_url = strField(b, "repo_url", 200);
  if (!REPO_URL.test(repo_url)) throw malformed("repo_url must look like https://github.com/<owner>/<repo>.");

  // A credential opens one run. Asking again with the same session_id is a lost reply: the same run.
  if (auth.run_id !== null) {
    const opened = await ctx.db.getRun(auth.member, auth.run_id);
    if (opened !== null && opened.session_id === session_id) return json(200, runView(opened));
    throw conflict("This credential already opened a run; run `goalcup continue`.");
  }

  if (ctx.cfg.githubCheck) await requirePublicGithubRepo(ctx, repo_url);

  const now = ctx.now();
  const result = await ctx.db.openRun({
    member_id: auth.member,
    event_id: ctx.cfg.eventId,
    run_id: newId("r"),
    entry_id: newId("e"),
    official_start: formatTime(now), // the server's clock, not the participant's
    deadline_at: formatTime(addSeconds(now, ctx.cfg.runSeconds)),
    connector_id,
    manifest_sha256,
    connector_dir_sha256,
    workspace,
    session_id,
    harness_version,
    tracer_version,
    repo_url,
  });
  if (result.kind === "conflict") throw conflict("This account already has an open run.");
  if (result.kind === "forbidden") throw new ApiError(403, "forbidden", result.message);

  await ctx.db.bindCredential(auth.credential, result.run.run_id, boundCredentialExpiry(result.run.deadline_at));
  return json(result.kind === "created" ? 201 : 200, runView(result.run));
}

const runView = ({ run_id, entry_id, official_start, deadline_at }: RunRow) => ({ run_id, entry_id, official_start, deadline_at });

/** GitHub says the repository exists and is public. Failures to ask are 502, so the tracer retries. */
async function requirePublicGithubRepo(ctx: Ctx, repoUrl: string): Promise<void> {
  const path = repoUrl.slice("https://github.com/".length); // owner/repo, shape already checked
  const headers: Record<string, string> = {
    accept: "application/vnd.github+json",
    "user-agent": "goalcup-upload-worker",
    "x-github-api-version": "2022-11-28",
  };
  if (ctx.env.GITHUB_TOKEN) headers.authorization = `Bearer ${ctx.env.GITHUB_TOKEN}`;
  let res: Response;
  try {
    res = await fetch(`https://api.github.com/repos/${path}`, { headers, signal: AbortSignal.timeout(8000) });
  } catch {
    throw new ApiError(502, "upstream_error", "GitHub could not be reached to check the repository.");
  }
  if (res.status === 404) throw malformed("repo_url is not a public GitHub repository (GitHub says it does not exist).");
  if (!res.ok) throw new ApiError(502, "upstream_error", "GitHub could not check the repository right now.");
  const info = (await res.json()) as { private?: unknown };
  if (info.private !== false) throw malformed("repo_url must be a public GitHub repository.");
}

// ---- POST /v1/runs/{id}/receipt -----------------------------------------------------------

export async function issueReceipt(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const run = await requireRun(ctx, member, runId);
  const b = await readJson(request, JSON_BODY_MAX);
  const entry_id = b.entry_id;
  if (!isEntryId(entry_id)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const chain_head = hexField(b, "chain_head");
  if (entry_id !== run.entry_id) throw notFound("That entry does not belong to this run.");
  requireOpen(run);

  if (run.receipt !== null) {
    if (run.receipt.claims.chain_head !== chain_head) throw conflict("This run already has a receipt for a different chain_head.");
    return json(200, { start_receipt: run.receipt });
  }

  const receipt = await ctx.signer.signReceipt({
    entry_id: run.entry_id,
    connector_id: run.connector_id,
    workspace: run.workspace,
    session_id: run.session_id,
    started_at: run.official_start,
    deadline_at: run.deadline_at,
    manifest_sha256: run.manifest_sha256,
    chain_head,
    connector_dir_sha256: run.connector_dir_sha256,
  });
  const stored = await ctx.db.storeReceipt(member, runId, chain_head, receipt);
  switch (stored.kind) {
    case "stored":
    case "existing":
      return json(200, { start_receipt: stored.receipt });
    case "conflict":
      throw conflict("This run already has a receipt for a different chain_head.");
    case "closed":
      throw closed("This run is no longer open.");
  }
}

// ---- POST /v1/runs/{id}/abandon -----------------------------------------------------------

export async function abandonRun(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const run = await requireRun(ctx, member, runId);
  const b = await readJson(request, JSON_BODY_MAX);
  const reason = strField(b, "reason", 4000, true);
  if ([...reason].length > 500) throw malformed("reason is longer than 500 characters.");

  // A repeat of an abandon whose answer was lost is not an error.
  if (run.status === "abandoned") return json(200, { status: "abandoned" });
  if (run.status !== "open") throw closed(`This run is ${run.status}.`);

  const r = await ctx.db.abandonRun(member, runId, reason, formatTime(ctx.now()));
  switch (r) {
    case "abandoned":
      return json(200, { status: "abandoned" });
    case "has_chunks":
      throw conflict("Chunks are already stored for this run, so it cannot be abandoned.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("No such run for this credential.");
  }
}

// ---- GET /v1/runs/{id} --------------------------------------------------------------------

export async function getPosition(ctx: Ctx, member: string, runId: string): Promise<Response> {
  const r = await requireRun(ctx, member, runId);
  return json(200, {
    run_id: r.run_id,
    entry_id: r.entry_id,
    status: r.status,
    official_start: r.official_start,
    deadline_at: r.deadline_at,
    offset: r.offset,
    next_seq: r.next_seq,
    head: r.head,
    last_ack: r.last_ack,
    stop: isStopped(ctx, r),
    final: r.final,
  });
}

/** Seconds a tracer is told to wait when it must send its final heartbeat first. */
export const FINAL_RETRY_AFTER = 60;

// ---- POST /v1/runs/{id}/heartbeat ---------------------------------------------------------

export async function heartbeat(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const run = await requireRun(ctx, member, runId);
  const b = await readJson(request, JSON_BODY_MAX);
  const entry_id = b.entry_id;
  if (!isEntryId(entry_id)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const seq = uintField(b, "seq");
  const head = hexField(b, "head");
  const offset_local = uintField(b, "offset_local");
  if (typeof b.final !== "boolean") throw malformed("final must be true or false.");
  const repo = b.repo_head ?? null;
  if (repo !== null && (typeof repo !== "string" || !/^[0-9a-f]{40}$/.test(repo))) throw malformed("repo_head must be null or 40 lowercase hex digits.");
  // Optional facts about the recording. W2 stores them; for now only their shape is checked.
  if (b.recording !== undefined && !["ok", "gaps", "failed", "unknown"].includes(b.recording as string)) {
    throw malformed('recording must be "ok", "gaps", "failed" or "unknown".');
  }
  if (b.model_requests !== undefined) uintField(b, "model_requests");
  if (entry_id !== run.entry_id) throw notFound("That entry does not belong to this run.");
  // An abandoned run accepts nothing more, a final heartbeat included. A sealed one still takes the final.
  if (run.status === "abandoned") throw closed("This run is abandoned.");

  const result = await ctx.db.recordHeartbeat({
    member_id: member,
    run_id: runId,
    entry_id: run.entry_id,
    seq,
    head,
    offset_local,
    final: b.final,
    repo_head: b.final ? repo : null, // only the final heartbeat's repo head means anything
    at: formatTime(ctx.now()),
  });
  switch (result) {
    case "recorded":
    case "final_committed":
    case "final_existing":
      return json(200, { stop: isStopped(ctx, run), deadline_at: run.deadline_at, reason: null });
    case "final_conflict":
      throw conflict("A different end of the run is already committed, or the server already holds a different chain than this final heartbeat names.");
    case "closed":
      throw closed("This run is abandoned.");
    case "not_found":
      throw notFound("No such run for this credential.");
  }
}

function uintField(o: Record<string, unknown>, name: string): number {
  const v = o[name];
  if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) throw malformed(`${name} must be a non-negative whole number.`);
  return v;
}

// ---- POST /v1/runs/{id}/seal --------------------------------------------------------------

export async function sealRun(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const run = await requireRun(ctx, member, runId);
  const b = await readJson(request, SEAL_BODY_MAX);
  const entry_id = b.entry_id;
  if (!isEntryId(entry_id)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const sealBytes = takeDocument(b, "seal_json");
  const baselineBytes = takeDocument(b, "baseline_json");
  if (entry_id !== run.entry_id) throw notFound("That entry does not belong to this run.");

  const sealSha = await sha256Hex(sealBytes);
  const baselineSha = await sha256Hex(baselineBytes);

  // Already sealed: same documents is a repeat (200), different documents is a conflict.
  if (run.status === "sealed") {
    if (run.seal_sha256 === sealSha && run.baseline_sha256 === baselineSha) return json(200, { status: "sealed" });
    throw conflict("This run is sealed with different documents.");
  }
  if (run.status !== "open") throw closed(`This run is ${run.status}.`);

  // After the deadline the seal is accepted only for the end the final heartbeat committed, and
  // only once the server holds the chain all the way to it. (The chain cannot grow past the
  // committed end, so a check against this snapshot cannot be overtaken.)
  // A committed end binds the seal before the deadline too.
  if (isStopped(ctx, run) && run.final === null) {
    throw new ApiError(503, "awaiting_final", "Send the final heartbeat before sealing a run past its deadline.", { "retry-after": String(FINAL_RETRY_AFTER) });
  }
  if (run.final !== null && (run.next_seq - 1 !== run.final.seq || run.head !== run.final.head)) {
    throw conflict("The server does not hold the chain up to the end the final heartbeat committed yet; read the position and upload the rest.");
  }

  // Store the documents first, under names that contain their hash (so they can never be
  // overwritten with different content), then record the seal.
  const prefix = `traces/${run.event_id}/${run.run_id}/${run.entry_id}`;
  const sealKey = `${prefix}/seal-${sealSha}.json`;
  const baselineKey = `${prefix}/baseline-${baselineSha}.json`;
  await putDocument(ctx, sealKey, sealBytes, sealSha);
  await putDocument(ctx, baselineKey, baselineBytes, baselineSha);

  const r = await ctx.db.sealRun({
    member_id: member,
    run_id: runId,
    entry_id: run.entry_id,
    seal_sha256: sealSha,
    baseline_sha256: baselineSha,
    seal_key: sealKey,
    baseline_key: baselineKey,
    sealed_at: formatTime(ctx.now()),
  });
  switch (r) {
    case "sealed":
    case "already_sealed":
      return json(200, { status: "sealed" });
    case "conflict":
      throw conflict("This run is sealed with different documents.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("No such run for this credential.");
  }
}

/** Create-only put; "already there" is fine because the name contains the content's hash. */
async function putDocument(ctx: Ctx, key: string, bytes: Uint8Array, sha256: string): Promise<void> {
  try {
    await ctx.env.TRACES.put(key, bytes, {
      sha256,
      httpMetadata: { contentType: "application/json" },
      onlyIf: { etagDoesNotMatch: "*" },
    });
  } catch (e) {
    console.error(`R2 put ${key} failed: ${String(e)}`);
    throw new ApiError(502, "upstream_error", "Storage could not keep the documents right now.");
  }
}
