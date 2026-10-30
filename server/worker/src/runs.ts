// Handlers for the run-level calls: open, continue, receipt, abandon, position, heartbeat, and the
// seal the server makes by itself. The chunk upload is in chunks.ts. Contract: docs/UPLOAD-API.md.

import type { Auth } from "./auth.ts";
import type { Ctx } from "./context.ts";
import { type EntryRow, type MachineFacts, type RecordingState, type RunMode, type Counts, type HeartbeatVitals, type RunRow, boundCredentialExpiry, endsAt, isPastClose, isPastEnd, isReplaced, statusAt } from "./db.ts";
import { ApiError, closed, conflict, json, malformed, notFound, tooLarge } from "./errors.ts";
import { githubHeaders } from "./github.ts";
import {
  REPO_URL,
  addSeconds,
  asObject,
  boolField,
  formatTime,
  hexField,
  isEntryId,
  newId,
  proxyVersionField,
  readBodyCapped,
  strField,
  versionTriple,
} from "./util.ts";

const JSON_BODY_MAX = 64 * 1024;

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

/** A run and one of its entries. */
export interface Found {
  run: RunRow;
  entry: EntryRow;
}

/**
 * The caller's own run and the entry named (or, with none named, the current one), or 404. A run
 * that belongs to someone else is indistinguishable from none, and so is an entry of another run.
 * A run that is over is closed here, the first time anyone looks at it once it is: `sealed` if its
 * current entry is sealed, else `incomplete` after the close time.
 */
export async function requireRun(ctx: Ctx, member: string, runId: string, entryId: string | null = null): Promise<Found> {
  const found = await ctx.db.getRun(member, runId, entryId);
  if (found === null) throw notFound("No such run for this credential.");
  if (found.entry === null) throw notFound("That entry does not belong to this run.");
  let run = found.run;
  const status = statusAt(run, ctx.now(), ctx.cfg.closeAfterSeconds);
  if (status !== run.status) run = { ...run, status: (await ctx.db.closeRun(member, runId, formatTime(ctx.now()))) ?? status };
  return { run, entry: found.entry };
}

/** 410 unless the run still takes uploads: it is not abandoned, and it has not been over for the close time. */
export function requireAccepting(ctx: Ctx, run: RunRow): void {
  if (run.status === "abandoned") throw closed("This run is abandoned.");
  if (isPastClose(run, ctx.now(), ctx.cfg.closeAfterSeconds)) throw closed("This run is closed.");
}

/** [requireAccepting], and the entry is not sealed. */
export function requireOpen(ctx: Ctx, run: RunRow, entry: EntryRow): void {
  requireAccepting(ctx, run);
  if (entry.sealed_at !== null) throw closed("This entry is sealed.");
}

/**
 * Whole seconds from the server's clock now to the run's `deadline_at`, rounded up, and 0 once it
 * has come. goalcup counts down from this and never from its own machine's date, so it
 * is on every answer that tells goalcup about the deadline: open, continue and heartbeat.
 */
export function secondsLeft(run: { deadline_at: string }, now: Date): number {
  return Math.max(0, Math.ceil((Date.parse(run.deadline_at) - now.getTime()) / 1000));
}

/** A heartbeat's answer: is the run over now, and if so why (a later start replaced it, or its deadline came)? */
function answerStop(ctx: Ctx, run: RunRow) {
  const now = ctx.now();
  const stop = isPastEnd(run, now);
  const reason = !stop ? null : isReplaced(run) ? "replaced" : "deadline";
  return { stop, deadline_at: run.deadline_at, seconds_left: secondsLeft(run, now), reason };
}

// ---- D70: the minimum goalcup version, and --dev ------------------------------------------

export const INSTALL_LINE = "curl -fsSL https://github.com/agentcommunity/goal-cup-proxy/releases/latest/download/install.sh | sh";
const DEV_COMPETES = "A --dev run never enters the competition. Start without --dev.";

/** `a` sorts before `b`, comparing major, then minor, then patch as numbers. */
function isBelow(a: [number, number, number], b: [number, number, number]): boolean {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i]! < b[i]!;
  return false;
}

/**
 * A start in any mode (a device request or an open) from a goalcup older than MIN_COMPETITION_VERSION
 * is 403, and from one whose version is unknown while a minimum is set. A minimum that is not
 * `x.y.z` is 500 misconfigured, at these two calls only. `continue` is never checked (D70): a run
 * already open finishes on the goalcup it began with. `sent` is the version string as the client sent it.
 */
export function requireMinimumVersion(ctx: Ctx, sent: string): void {
  const min = ctx.cfg.minCompetitionVersion;
  if (min.kind === "none") return;
  if (min.kind === "malformed") {
    console.error("misconfigured: MIN_COMPETITION_VERSION is not x.y.z");
    throw new ApiError(500, "misconfigured", "The server's minimum goalcup version is not set correctly. Try again later.");
  }
  const have = versionTriple(sent);
  const older = have === null || isBelow(have, min.v);
  if (older) throw new ApiError(403, "forbidden", `This goalcup (${sent}) is older than this server accepts (${min.v.join(".")}). Update it: ${INSTALL_LINE}`);
}

/** `dev` on a run in this mode: 403 for a competition run. Practice and test runs count for nothing, so anyone may use it there. */
function requireDevAllowed(mode: RunMode): void {
  if (mode === "competition") throw new ApiError(403, "forbidden", DEV_COMPETES);
}

// ---- POST /v1/runs ------------------------------------------------------------------------

const OPENED_ALREADY = "This credential already opened a run; run `goalcup continue`.";
const MACHINE_MAX_BYTES = 4096;

/** The facts about a connector and a session that an open or a continue call sends. */
function connectorFields(b: Record<string, unknown>) {
  const session_id = strField(b, "session_id", 128);
  if (!/^[A-Za-z0-9._:-]+$/.test(session_id)) throw malformed("session_id has characters a session id never has.");
  return {
    connector_id: strField(b, "connector_id", 128),
    ...connectorDigest(b),
    workspace: strField(b, "workspace", 4096),
    session_id,
    harness_version: strField(b, "harness_version", 64),
    tracer_version: proxyVersionField(b),
  };
}

/**
 * `connector_sha256`: the SHA-256 of the connector file the client ran. The database keeps two
 * NOT NULL digest columns from before; the one value goes into both, so no migration is needed.
 */
function connectorDigest(b: Record<string, unknown>): { manifest_sha256: string; connector_dir_sha256: string } {
  const digest = hexField(b, "connector_sha256");
  return { manifest_sha256: digest, connector_dir_sha256: digest };
}

/** `machine`, if sent: a flat object of strings, numbers, booleans and nulls, at most 4 KiB. Null is absent. */
function machineField(b: Record<string, unknown>): MachineFacts | null {
  if (b.machine === undefined || b.machine === null) return null;
  const machine = asObject(b.machine, "machine");
  for (const [key, value] of Object.entries(machine)) {
    const flat = value === null || ["string", "number", "boolean"].includes(typeof value);
    if (!flat) throw malformed("machine must be a flat object of strings, numbers, booleans and nulls.");
    if (!key.isWellFormed() || (typeof value === "string" && !value.isWellFormed())) throw malformed("machine is not well-formed Unicode.");
  }
  if (new TextEncoder().encode(JSON.stringify(machine)).length > MACHINE_MAX_BYTES) throw malformed("machine is over 4 KiB.");
  return machine as MachineFacts;
}

/** What Cloudflare says about the connection: kept on the run, never shown, never in a key or a log. */
function clientFacts(request: Request): { client_ip: string | null; client_country: string | null } {
  const ip = request.headers.get("cf-connecting-ip");
  const country = request.cf?.country;
  return { client_ip: ip === null || ip === "" ? null : ip.slice(0, 64), client_country: typeof country === "string" ? country.slice(0, 8) : null };
}

export async function openRun(ctx: Ctx, auth: Auth, request: Request): Promise<Response> {
  const b = await readJson(request, JSON_BODY_MAX);
  const fields = connectorFields(b);
  const repo_url = strField(b, "repo_url", 200);
  if (!REPO_URL.test(repo_url)) throw malformed("repo_url must look like https://github.com/<owner>/<repo>.");
  const machine = machineField(b);
  const dev = boolField(b, "dev", false);

  // A credential opens one run. Asking again with the same session_id is a lost reply: the same run.
  if (auth.run_id !== null) {
    // (The session of the open call is segment 0's, whatever `continue` has added since.)
    let opened = await ctx.db.getRun(auth.member, auth.run_id, null);
    if (opened !== null && opened.entry?.segment !== 0) opened = await ctx.db.getRun(auth.member, auth.run_id, opened.run.first_entry_id);
    if (opened?.entry != null && opened.entry.session_id === fields.session_id) return json(200, runView(ctx, opened.run, opened.entry));
    throw conflict(OPENED_ALREADY);
  }

  requireMinimumVersion(ctx, fields.tracer_version);
  if (dev) requireDevAllowed(auth.mode);

  // A practice run judges nothing, so its repository is never asked about: 0.7 practice runs all
  // name one shared repository, and one commit there must not stop every practice start.
  if (ctx.cfg.githubCheck && auth.mode !== "practice") {
    await requirePublicGithubRepo(ctx, repo_url);
    await requireEmptyGithubRepo(ctx, repo_url);
  }

  const now = ctx.now();
  const practice = auth.mode === "practice";
  // A test run belongs to the test event; the database sets its length from that event's run_hours.
  if (auth.mode === "test" && ctx.cfg.testEventId === undefined) throw new ApiError(403, "forbidden", "This server opens no test runs.");
  const event_id = auth.mode === "test" ? ctx.cfg.testEventId! : practice ? ctx.cfg.practiceEventId : ctx.cfg.eventId;
  const deadline_at = formatTime(addSeconds(now, practice ? ctx.cfg.practiceRunSeconds : ctx.cfg.runSeconds));
  const result = await ctx.db.openRun({
    member_id: auth.member,
    event_id,
    mode: auth.mode,
    run_id: newId("r"),
    entry_id: newId("e"),
    official_start: formatTime(now), // the server's clock, not the participant's
    deadline_at,
    ...fields,
    repo_url,
    machine,
    ...clientFacts(request),
    credential: auth.credential,
    credential_expires_at: boundCredentialExpiry(deadline_at),
  });
  if (result.kind === "conflict") throw conflict(OPENED_ALREADY);
  if (result.kind === "forbidden") throw new ApiError(403, "forbidden", result.message);
  return json(result.kind === "created" ? 201 : 200, runView(ctx, result.run, result.entry));
}

const runView = (ctx: Ctx, run: RunRow, { entry_id }: EntryRow) => ({
  run_id: run.run_id,
  entry_id,
  official_start: run.official_start,
  deadline_at: run.deadline_at,
  seconds_left: secondsLeft(run, ctx.now()),
});

// ---- POST /v1/runs/{id}/continue ----------------------------------------------------------

export async function continueRun(ctx: Ctx, auth: Auth, runId: string, request: Request): Promise<Response> {
  const b = await readJson(request, JSON_BODY_MAX);
  const previous_entry_id = b.previous_entry_id;
  if (!isEntryId(previous_entry_id)) throw malformed("previous_entry_id must be e and 16 lowercase hex digits.");
  const fields = connectorFields(b);
  if (boolField(b, "dev", false)) {
    // The run's own mode, not the credential's: a continued run keeps the rules it began under.
    const found = await ctx.db.getRun(auth.member, runId, null);
    if (found === null) throw notFound("No such run for this credential.");
    requireDevAllowed(found.run.mode);
  }
  const result = await ctx.db.continueRun({
    member_id: auth.member,
    run_id: runId,
    previous_entry_id,
    entry_id: newId("e"),
    ...fields,
    ...clientFacts(request),
    at: formatTime(ctx.now()),
  });
  switch (result.kind) {
    case "created":
    case "existing":
      return json(result.kind === "created" ? 201 : 200, {
        entry_id: result.entry.entry_id,
        segment: result.entry.segment,
        official_start: result.run.official_start,
        deadline_at: result.run.deadline_at,
        seconds_left: secondsLeft(result.run, ctx.now()),
      });
    case "conflict":
      throw conflict("previous_entry_id is not the latest entry of this run; read the position.");
    case "closed":
      throw closed("This run is over; there is nothing left to continue.");
    case "not_found":
      throw notFound("No such run for this credential.");
  }
}

/** GitHub says the repository exists and is public. Failures to ask are 502, so goalcup retries. */
async function requirePublicGithubRepo(ctx: Ctx, repoUrl: string): Promise<void> {
  const path = repoUrl.slice("https://github.com/".length); // owner/repo, shape already checked
  const headers = githubHeaders(ctx.env);
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

const REPO_NOT_EMPTY = "The repository must be empty when the run opens. Create a new one with no README, .gitignore or license, and start again.";

/**
 * The repository has no commits yet (OD7). `GET /repos/<owner>/<repo>/commits?per_page=1` answers 409
 * for an empty repository and 200 for one with a commit. Any other answer, or no answer, is 502 so
 * goalcup retries; a GitHub blip never refuses a player.
 */
async function requireEmptyGithubRepo(ctx: Ctx, repoUrl: string): Promise<void> {
  const path = repoUrl.slice("https://github.com/".length); // owner/repo, shape already checked
  let res: Response;
  try {
    res = await fetch(`https://api.github.com/repos/${path}/commits?per_page=1`, { headers: githubHeaders(ctx.env), signal: AbortSignal.timeout(8000) });
  } catch {
    throw new ApiError(502, "upstream_error", "GitHub could not be reached to check the repository.");
  }
  await res.body?.cancel().catch(() => {});
  if (res.status === 409) return;
  if (res.status === 200) throw malformed(REPO_NOT_EMPTY);
  throw new ApiError(502, "upstream_error", "GitHub could not check the repository right now.");
}

// ---- POST /v1/runs/{id}/receipt -----------------------------------------------------------

export async function issueReceipt(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const b = await readJson(request, JSON_BODY_MAX);
  const entry_id = b.entry_id;
  if (!isEntryId(entry_id)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const chain_head = hexField(b, "chain_head");
  const { run, entry } = await requireRun(ctx, member, runId, entry_id);
  requireOpen(ctx, run, entry);

  if (entry.receipt !== null) {
    if (entry.receipt.claims.chain_head !== chain_head) throw conflict("This entry already has a receipt for a different chain_head.");
    return json(200, { start_receipt: entry.receipt });
  }

  // The entry's own connector facts and session, the run's start and deadline.
  const receipt = await ctx.signer.signReceipt({
    entry_id: entry.entry_id,
    connector_id: entry.connector_id,
    workspace: entry.workspace,
    session_id: entry.session_id,
    started_at: run.official_start,
    deadline_at: run.deadline_at,
    manifest_sha256: entry.manifest_sha256,
    chain_head,
    connector_dir_sha256: entry.connector_dir_sha256,
  });
  const stored = await ctx.db.storeReceipt(member, runId, entry.entry_id, chain_head, receipt);
  switch (stored.kind) {
    case "stored":
    case "existing":
      return json(200, { start_receipt: stored.receipt });
    case "conflict":
      throw conflict("This entry already has a receipt for a different chain_head.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("That entry does not belong to this run.");
  }
}

// ---- POST /v1/runs/{id}/abandon -----------------------------------------------------------

export async function abandonRun(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const { run } = await requireRun(ctx, member, runId);
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

// ---- GET /v1/runs/{id}[?entry_id=…] -------------------------------------------------------

export async function getPosition(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const asked = new URL(request.url).searchParams.get("entry_id");
  if (asked !== null && !isEntryId(asked)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const { run, entry } = await requireRun(ctx, member, runId, asked);
  return json(200, {
    run_id: run.run_id,
    entry_id: entry.entry_id,
    segment: entry.segment,
    status: run.status,
    official_start: run.official_start,
    deadline_at: run.deadline_at,
    ends_at: endsAt(run),
    offset: entry.offset,
    next_seq: entry.next_seq,
    head: entry.head,
    last_ack: entry.last_ack,
    stop: isPastEnd(run, ctx.now()),
    final: entry.final,
    sealed: entry.sealed_at !== null,
  });
}

/** Seconds goalcup is told to wait when it must send its final heartbeat first. */
export const FINAL_RETRY_AFTER = 60;

// ---- POST /v1/runs/{id}/heartbeat ---------------------------------------------------------

export async function heartbeat(ctx: Ctx, member: string, runId: string, request: Request): Promise<Response> {
  const b = await readJson(request, JSON_BODY_MAX);
  const entry_id = b.entry_id;
  if (!isEntryId(entry_id)) throw malformed("entry_id must be e and 16 lowercase hex digits.");
  const seq = uintField(b, "seq");
  const head = hexField(b, "head");
  const offset_local = uintField(b, "offset_local");
  if (typeof b.final !== "boolean") throw malformed("final must be true or false.");
  const repo = b.repo_head ?? null;
  if (repo !== null && (typeof repo !== "string" || !/^[0-9a-f]{40}$/.test(repo))) throw malformed("repo_head must be null or 40 lowercase hex digits.");
  // Optional facts; null is absent, as for repo_head.
  if (b.recording != null && !["ok", "gaps", "failed", "unknown"].includes(b.recording as string)) {
    throw malformed('recording must be "ok", "gaps", "failed" or "unknown".');
  }
  const model_requests = b.model_requests == null ? null : uintField(b, "model_requests");
  const vitals = countsField(b, runId);
  const { run, entry } = await requireRun(ctx, member, runId, entry_id);
  // An abandoned or closed run accepts nothing more, a final heartbeat included.
  requireAccepting(ctx, run);

  const result = await ctx.db.recordHeartbeat({
    member_id: member,
    run_id: runId,
    entry_id: entry.entry_id,
    seq,
    head,
    offset_local,
    final: b.final,
    repo_head: b.final ? repo : null, // only the final heartbeat's repo head means anything
    recording: (b.recording ?? null) as RecordingState | null,
    model_requests,
    vitals,
    ...clientFacts(request),
    at: formatTime(ctx.now()),
  });
  switch (result) {
    case "recorded":
      return json(200, { ...answerStop(ctx, run), sealed: entry.sealed_at !== null });
    case "final_committed":
    case "final_existing": {
      // The end is committed at (seq, head): seal it now if the server already holds the chain up
      // to it. If not, the chunk that completes it seals it (chunks.ts).
      const sealed = entry.sealed_at !== null || (await trySeal(ctx, member, runId, entry.entry_id, head));
      return json(200, { ...answerStop(ctx, run), sealed });
    }
    case "final_conflict":
      throw conflict("A different end of the run is already committed, or the server already holds a different chain than this final heartbeat names.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("No such run for this credential.");
  }
}

function uintField(o: Record<string, unknown>, name: string, label: string = name): number {
  const v = o[name];
  if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) throw malformed(`${label} must be a non-negative whole number.`);
  return v;
}

/** The most bytes a heartbeat's `counts` may take as JSON, and the longest string value in it. */
const COUNTS_MAX_BYTES = 2048;
const COUNT_STRING_MAX_BYTES = 64;

/**
 * `counts`, if sent, as the heartbeat stores it: `{"counts": <counts>}`. Nested under one key, so no
 * key the client chose sits at the top level of the stored object. The server fixes no key list;
 * valid is a flat object whose values are whole numbers, strings of at most 64 bytes, booleans or
 * nulls, at most 2 KiB as JSON. Counts never cost a heartbeat (it is what keeps the run alive, D64):
 * a bad `counts` is dropped, the heartbeat stores `{"counts_dropped": <its length as JSON>}`, and one
 * log line `counts-dropped <run_id>` says so, without the value. Null or absent: nothing (null).
 */
function countsField(b: Record<string, unknown>, runId: string): HeartbeatVitals | null {
  const counts = b.counts;
  if (counts == null) return null;
  const size = utf8Length(JSON.stringify(counts));
  if (size <= COUNTS_MAX_BYTES && isFlatCounts(counts)) return { counts };
  console.warn(`counts-dropped ${runId}`);
  return { counts_dropped: size };
}

function isFlatCounts(v: unknown): v is Counts {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  for (const [key, value] of Object.entries(v)) {
    if (!key.isWellFormed()) return false;
    const ok =
      value === null ||
      typeof value === "boolean" ||
      (typeof value === "number" && Number.isSafeInteger(value) && value >= 0) ||
      (typeof value === "string" && value.isWellFormed() && utf8Length(value) <= COUNT_STRING_MAX_BYTES);
    if (!ok) return false;
  }
  return true;
}

const utf8Length = (text: string): number => new TextEncoder().encode(text).length;

// ---- the seal -----------------------------------------------------------------------------

/**
 * Seal the entry when the server holds the whole chain: the final heartbeat has committed the end
 * at `head`, and the chain the server holds reaches it. Called by the final heartbeat (with the head
 * it committed) and by the chunk that ends at the committed end (with that chunk's head), so
 * whichever comes second seals. The database checks both facts again under its lock, so a race is
 * safe and a head that is not the committed one never seals. The two digest columns get the final
 * head; there are no seal documents, so the two key columns stay null. True when the entry is
 * sealed now. A failure here is not the caller's: it is logged and answered `sealed: false`, and the
 * client repeats the final heartbeat until an answer says `sealed: true`.
 */
export async function trySeal(ctx: Ctx, member: string, runId: string, entryId: string, head: string): Promise<boolean> {
  try {
    const r = await ctx.db.sealRun({
      member_id: member,
      run_id: runId,
      entry_id: entryId,
      seal_sha256: head,
      baseline_sha256: head,
      seal_key: null,
      baseline_key: null,
      sealed_at: formatTime(ctx.now()),
    });
    return r.kind === "sealed" || r.kind === "already_sealed"; // no_final, chain_behind: not yet
  } catch (e) {
    console.error(`seal-failed ${runId}: ${e instanceof Error ? e.message : String(e)}`);
    return false;
  }
}
