// The Worker's view of its database: one method per step of an API call. Two implementations
// (db-supabase.ts for production, db-local.ts for wrangler dev and tests) must behave the same;
// test/integration.test.ts exercises the local one, and the SQL in sql/ mirrors it.
//
// Rules every implementation keeps:
//   * A run belongs to one member. Asking for another member's run gives null ("not found"),
//     never a different answer, so a credential cannot probe for other members' runs.
//   * Codes and credentials are stored as sha256 only. The one exception is the instant a
//     credential is minted: `pollDevice` returns it once, and keeps its hash.
//   * Every write is atomic: it either applies fully against the state the caller saw, or it
//     reports why not and changes nothing.

import { addSeconds, formatTime } from "./util.ts";

/** Twenty-four hours after a run ends the server closes it: no upload is accepted after that. */
export const CLOSE_AFTER_SECONDS = 24 * 3600;

/** `abandoned` is only read: rows from before 1.0 may carry it, and nothing sets it any more. */
export type RunStatus = "open" | "sealed" | "incomplete" | "abandoned";
/** `test`: an organizer's test run (allowlisted accounts only, its own event and R2 prefix, deleted in one go). */
export type RunMode = "competition" | "practice" | "test";

/** The instant a run is over: `deadline_at`, or earlier if a later start replaced it. (Times compare as text.) */
export function endsAt(run: { deadline_at: string; replaced_at: string | null }): string {
  return run.replaced_at !== null && run.replaced_at < run.deadline_at ? run.replaced_at : run.deadline_at;
}

/** A later start replaced the run before its deadline. */
export function isReplaced(run: { deadline_at: string; replaced_at: string | null }): boolean {
  return run.replaced_at !== null && run.replaced_at < run.deadline_at;
}

/** True once the server's clock is at or past the end of the run: the run is over. */
export function isPastEnd(run: { deadline_at: string; replaced_at: string | null }, now: Date): boolean {
  return now.getTime() >= new Date(endsAt(run)).getTime();
}

/** True once `now` is more than `afterSeconds` past the end. (Only tests pass a shorter time.) */
export function isPastClose(run: { deadline_at: string; replaced_at: string | null }, now: Date, afterSeconds: number = CLOSE_AFTER_SECONDS): boolean {
  return now.getTime() > addSeconds(new Date(endsAt(run)), afterSeconds).getTime();
}

/**
 * What the run's status is at `now`. A run is `sealed` once it is over and its current entry is
 * sealed, and `incomplete` once it has been over for the close time with that entry unsealed.
 * Words already written stay as they are.
 */
export function statusAt(run: RunRow, now: Date, closeAfterSeconds: number = CLOSE_AFTER_SECONDS): RunStatus {
  if (run.status !== "open" || !isPastEnd(run, now)) return run.status;
  if (run.current_sealed_at !== null) return "sealed";
  return isPastClose(run, now, closeAfterSeconds) ? "incomplete" : "open";
}

/** A run credential that has not opened a run yet lasts this long after the approval. */
export const UNBOUND_CREDENTIAL_SECONDS = 3600;
/** Once it has opened a run, a credential lasts until this long after the run's `deadline_at`. */
export const BOUND_CREDENTIAL_EXTRA_SECONDS = 24 * 3600;

/** The `expires_at` of a credential that has opened a run with this deadline. */
export function boundCredentialExpiry(deadlineAt: string): string {
  return formatTime(addSeconds(new Date(deadlineAt), BOUND_CREDENTIAL_EXTRA_SECONDS));
}

/** The first record of a v5 chain (the baseline) has seq 0; a fresh run's next_seq is 0. */
export const FIRST_SEQ = 0;
/** The head before any record: 64 zeros, the chain's genesis. */
export const GENESIS = "0".repeat(64);

/** The end of the run, committed by the final heartbeat: the last record the server will ever accept. */
export interface FinalRow {
  seq: number;
  head: string;
}

/** A run: what the account started. Its entries (segments) hold the chain; see `EntryRow`. */
export interface RunRow {
  run_id: string;
  event_id: string;
  member_id: string;
  mode: RunMode;
  /** As stored; use `statusAt` for what it is now. */
  status: RunStatus;
  official_start: string;
  deadline_at: string;
  /** Set when a later competition start by the account replaced this competition run: the run ends then. */
  replaced_at: string | null;
  repo_url: string;
  /** The entry of segment 0, the one `POST /v1/runs` made. */
  first_entry_id: string;
  /** The entry of the highest segment. */
  current_entry_id: string;
  /** When that entry was sealed, or null. */
  current_sealed_at: string | null;
  /**
   * What the run holds in R2 so far, over all its entries: the compressed bytes (each committed
   * chunk's Content-Length) and the number of committed chunks. There is no per-run size limit;
   * the chunk path logs each 10 GiB a run crosses.
   */
  stored_bytes: number;
  chunk_count: number;
}

/** One segment of a run: its own chain and seal. Segment 0 is made by `POST /v1/runs`. */
export interface EntryRow {
  entry_id: string;
  run_id: string;
  segment: number;
  // What the open (or continue) call sent.
  connector_id: string;
  manifest_sha256: string;
  connector_dir_sha256: string;
  workspace: string;
  session_id: string;
  // Where the upload stands.
  offset: number;
  next_seq: number;
  head: string;
  /** Set once, by the final heartbeat. */
  final: FinalRow | null;
  // Set once.
  seal_sha256: string | null;
  baseline_sha256: string | null;
  sealed_at: string | null;
}

/** A run and one of its entries (the one asked for, or the current one). */
export interface RunAndEntry {
  run: RunRow;
  entry: EntryRow;
}

export interface ChunkRecord {
  entry_id: string;
  offset: number;
  length: number;
  sha256: string; // of the compressed body
  first_seq: number;
  last_seq: number;
  prev: string;
  head: string;
  r2_key: string;
  received_at: string;
}

/** The facts about the machine goalcup sends: a flat object of strings, numbers, booleans and nulls. */
export type MachineFacts = Record<string, string | number | boolean | null>;

export interface OpenRunArgs {
  member_id: string;
  event_id: string;
  mode: RunMode;
  run_id: string; // freshly minted
  entry_id: string;
  official_start: string;
  deadline_at: string;
  connector_id: string;
  manifest_sha256: string;
  connector_dir_sha256: string;
  workspace: string;
  session_id: string;
  harness_version: string;
  tracer_version: string;
  repo_url: string;
  machine: MachineFacts | null;
  /** Server-side facts about the connection (never shown to anyone). */
  client_ip: string | null;
  client_country: string | null;
  /** sha256 of the credential that opens the run, and when it ends once bound. Bound in the same step. */
  credential: string;
  credential_expires_at: string;
}

/**
 * `existing`: the credential already opened a run whose first entry has this `session_id` (a retry
 * that raced the original); that run and entry. `conflict`: the credential is unknown or opened
 * another run; nothing was created. `forbidden`: terms not accepted, or outside the start window.
 */
export type OpenRunResult =
  | ({ kind: "created" | "existing" } & RunAndEntry)
  | { kind: "conflict" }
  | { kind: "forbidden"; message: string };

export interface ContinueRunArgs {
  member_id: string;
  run_id: string;
  previous_entry_id: string;
  entry_id: string; // freshly minted; ignored if an existing entry is returned
  connector_id: string;
  manifest_sha256: string;
  connector_dir_sha256: string;
  workspace: string;
  session_id: string;
  harness_version: string;
  tracer_version: string;
  client_ip: string | null;
  client_country: string | null;
  at: string;
}

/**
 * `existing`: the same call again (the current entry came from this `previous_entry_id` and
 * `session_id`). `conflict`: `previous_entry_id` is not the current entry. `closed`: the run is
 * over, abandoned or closed.
 */
export type ContinueRunResult = ({ kind: "created" | "existing" } & RunAndEntry) | { kind: "conflict" | "closed" | "not_found" };

export interface CommitChunkArgs {
  member_id: string;
  run_id: string;
  chunk: ChunkRecord;
  /** The compressed bytes this chunk put in R2 (its Content-Length): added to the entry's `stored_bytes` with the chunk. */
  stored_bytes: number;
}
/**
 * `conflict`: offset, prev or first_seq no longer match the entry (someone advanced it first).
 * `beyond_final`: the chunk goes past the end the final heartbeat committed.
 * `closed`: the run is abandoned or incomplete, or the entry is sealed.
 */
export type CommitChunkResult = "committed" | "conflict" | "beyond_final" | "closed" | "not_found";

/** How goalcup says its own recording is going. */
export type RecordingState = "ok" | "gaps" | "failed" | "unknown";

/**
 * A heartbeat's written counts (docs/UPLOAD-API.md, heartbeat): a flat object the server checks for
 * shape and size only and never interprets. It fixes no key list.
 */
export type Counts = Record<string, number | string | boolean | null>;

/**
 * What a heartbeat stores in its `vitals` column and as the run's `last_vitals` (the column names
 * stay; renaming them would be a migration): the counts nested under one key, or, when the
 * heartbeat's `counts` was not valid, only its length as JSON. Nothing the client names ever sits at
 * the top level of this object.
 */
export type HeartbeatVitals = { counts: Counts } | { counts_dropped: number };

export interface HeartbeatArgs {
  member_id: string;
  run_id: string;
  entry_id: string;
  seq: number;
  head: string;
  offset_local: number;
  final: boolean;
  repo_head: string | null; // 40 hex, final only
  /** Optional facts the heartbeat carries; null when it did not send them. */
  recording: RecordingState | null;
  model_requests: number | null;
  /** Null when the heartbeat sent no `counts`. */
  vitals: HeartbeatVitals | null;
  /** Server-side facts about the connection (never shown to anyone). */
  client_ip: string | null;
  client_country: string | null;
  at: string;
}
/**
 * `recorded`: an ordinary heartbeat was noted. Either kind sets the run's last heartbeat and its
 * latest IP and country (a missing one keeps the last). A `recording` of `gaps` or `failed` flags
 * the run (once, the first wins). `model_requests` is kept per entry (a continued process counts
 * from zero): the first one an entry reports, and any above that entry's stored value, stores it
 * and moves the run's last model request to `at`; a lower or equal one changes nothing. `vitals`
 * (`HeartbeatVitals`) are kept, as given, on the heartbeat and as the run's latest (none keeps the
 * last); nothing in them starts a disk pause, because the counts sit one level down. Anything else
 * changes nothing. A repeat of the committed final heartbeat (`final_existing`) records nothing:
 * no heartbeat row. `final_committed` / `final_existing`: the final
 * heartbeat's (seq, head) is now / already was the committed end. `final_conflict`: a different end is committed, or the
 * chain the server holds already contradicts this one. `closed`: the run is abandoned or incomplete.
 */
export type HeartbeatResult = "recorded" | "final_committed" | "final_existing" | "final_conflict" | "closed" | "not_found";

export interface SealArgs {
  member_id: string;
  run_id: string;
  entry_id: string;
  seal_sha256: string;
  baseline_sha256: string;
  /** Null when the seal stores no documents of its own: the server seals by itself with the final head. */
  seal_key: string | null;
  baseline_key: string | null;
  sealed_at: string;
}
/**
 * `sealed` / `already_sealed` (same documents) carry the run's status after the seal: `sealed`
 * when this was the current entry and the run is over, else `open`. `conflict`: other documents.
 * `no_final` / `chain_behind`: the entry has no committed end yet, or the server does not hold
 * the chain up to it; checked in the same step as the seal.
 */
export type SealResult =
  | { kind: "sealed" | "already_sealed"; run_status: RunStatus }
  | { kind: "conflict" | "no_final" | "chain_behind" | "closed" | "not_found" };

export type DevicePurpose = "login" | "run";

export interface CreateDeviceArgs {
  device_sha256: string;
  /** sha256 of the user code without the dash, as `normalizeUserCode` gives it. */
  user_sha256: string;
  purpose: DevicePurpose;
  mode: RunMode;
  connector_id: string;
  tracer_version: string;
  created_at: string;
  expires_at: string;
}

/**
 * What a poll of a device request finds. `approved` is returned once: a run request carries the
 * freshly minted `credential_plain` (stored hashed) and its `expires_at`, a login request the
 * member's `handle`. Every later poll, and any poll at or after `expires_at`, is `expired`.
 */
export type DevicePoll =
  | { kind: "pending" }
  | { kind: "approved"; purpose: DevicePurpose; credential_plain?: string; expires_at?: string; handle?: string }
  | { kind: "denied" }
  | { kind: "expired" };

/** `expired`: the request ran out of time. `already_decided`: it was approved or denied before. */
/**
 * `forbidden`: the database refused the test approval (Supabase: the caller is not the staging
 * Worker, the request was not made on staging, or the member is not a synthetic test member).
 */
export type ApproveDeviceResult = "ok" | "not_found" | "expired" | "already_decided" | "forbidden";

/** What a run credential stands for. `run_id` is null until it opens a run. */
export interface Credential {
  member_id: string;
  run_id: string | null;
  mode: RunMode;
  expires_at: string;
}

/** A run as the cron reads it: the run, and what the heartbeats and the deadline added to it. */
export interface TickRun extends RunRow {
  /** The latest heartbeat of any of its entries. */
  last_heartbeat_at: string | null;
  /** When an entry's `model_requests` was first reported or last grew; null until a heartbeat reports it. */
  last_model_request_at: string | null;
  /** When a heartbeat first said the recording had `gaps` or `failed`, and which of the two. */
  flagged_at: string | null;
  flag_reason: "gaps" | "failed" | null;
  /** The judged GitHub commit (see `FinalCommitDetail`): 40 hex, `none` for a repository GitHub does not have, null until decided. */
  final_commit: string | null;
  /**
   * The start of the latest disk pause that is owed a disk_full email (see `HeartbeatResult`); null
   * until a heartbeat reports one.
   */
  disk_notice_at: string | null;
}

/**
 * How the judged commit was found (docs/UPLOAD-API.md, "The judged commit"): the last push to the
 * repository's default branch that GitHub timestamped at or before the run's `ends_at`.
 */
export interface FinalCommitDetail {
  /** When GitHub received the judged push (its activity `timestamp`); null when not known. */
  pushed_at: string | null;
  /** The default branch that was read; null when GitHub gave none. */
  branch: string | null;
  /** GitHub had a push to that branch after `ends_at` when it was asked (not judged); null when not known. */
  late_push: boolean | null;
  /** GitHub had a force push to that branch between the run's start and `ends_at`; null when not known. */
  force_push: boolean | null;
  /**
   * The commit was not read from the activity API, so an organizer reviews it by hand: it came from
   * the commit list (`commits?sha=<branch>&until=<ends_at>`, by commit date) or `commits/HEAD` (the
   * head when asked, which may be a later push), because the activity API was unavailable, listed
   * no update of the branch, or listed only updates after `ends_at` within its page limit; or it is
   * `none` because GitHub kept answering an error status for the grace hour.
   */
  head_fallback: boolean;
}

/** No detail: what a caller that passes none stores. */
export const NO_FINAL_DETAIL: Readonly<FinalCommitDetail> = Object.freeze({ pushed_at: null, branch: null, late_push: null, force_push: null, head_fallback: false });

/** A run that is over this long, and no longer open, is not looked at by the cron any more. */
export const TICK_LOOKBACK_SECONDS = 48 * 3600;

/** Every kind of run email; the database's notices CHECK constraint and its notice_text list the same. */
export const NOTICE_KINDS = ["started", "silence", "idle_3h", "idle_6h", "recording", "ended", "disk_full"] as const;
export type NoticeKind = (typeof NOTICE_KINDS)[number];

/** One row for the outbox: src/mail.ts sends it, once, with the text and address the database gives. */
export interface NoticeArgs {
  run_id: string;
  member_id: string;
  kind: NoticeKind;
  /** What makes this the same notice again: a notice is queued once per (run, kind, dedupe_key). */
  dedupe_key: string;
  payload: Record<string, unknown>;
  at: string;
}

/**
 * A queued notice claimed for sending: the database decided the address (null only for a member it
 * has no account for, such as a synthetic staging member), checked suppression, and wrote the email.
 */
export interface ClaimedNotice {
  id: number;
  kind: NoticeKind;
  run_id: string;
  to: string | null;
  subject: string;
  text: string;
}

/** How sending one claimed notice ended: `detail` is the provider's message id, the skip reason, or a short error. */
export type NoticeOutcome = "sent" | "skipped" | "failed";

export interface Db {
  /** Start a device request. The two hashes must be new. */
  createDevice(args: CreateDeviceArgs): Promise<void>;
  /** See `DevicePoll`. */
  pollDevice(deviceSha256: string, now: string): Promise<DevicePoll>;
  /** Decide a pending request by its user code (8 letters, upper case, no dash) as `member_id`. */
  approveDevice(userCode: string, memberId: string, approve: boolean, now: string): Promise<ApproveDeviceResult>;
  /** sha256 hex of a bearer credential -> what it stands for, or null if unknown or past `expires_at`. */
  credential(credentialSha256: string, now: string): Promise<Credential | null>;
  /**
   * Open a run (segment 0) for the account and bind the credential to it, in one atomic step.
   * A competition run replaces the account's other open competition run (`replaced_at` = this
   * run's `official_start`). Only competition replaces: a test run or a practice run replaces
   * nothing and is never replaced, so an operator's test runs can run side by side.
   */
  openRun(args: OpenRunArgs): Promise<OpenRunResult>;
  /** Add the next segment. See `ContinueRunResult`. */
  continueRun(args: ContinueRunArgs): Promise<ContinueRunResult>;
  /** The run, and the entry named (null if it is not this run's) or the current one when none is named. Null if the run is not this member's. */
  getRun(memberId: string, runId: string, entryId: string | null): Promise<{ run: RunRow; entry: EntryRow | null } | null>;
  /** Note a heartbeat; a final one commits the end of the run (once). */
  recordHeartbeat(args: HeartbeatArgs): Promise<HeartbeatResult>;
  /**
   * Write the run's close: an open run that is over with its current entry sealed becomes
   * `sealed`; one that has been over for the close time becomes `incomplete`. Returns the status
   * now (null if the run is not this member's).
   */
  closeRun(memberId: string, runId: string, at: string): Promise<RunStatus | null>;
  /** Every run that is open, and every run that ended within `TICK_LOOKBACK_SECONDS` before `now`, except abandoned ones. Oldest first. */
  runsForTick(now: string): Promise<TickRun[]>;
  /**
   * Queue notices, each unless one with the same run, kind and key exists, in one call. Returns how
   * many this call queued. A batch that fails is retried whole: the key makes that safe.
   */
  queueNotices(notices: NoticeArgs[]): Promise<number>;
  /**
   * Claim up to `limit` unsent notices for the mailer. A claim lasts 15 minutes, so a send that died
   * is claimed again; suppressed or address-less members are skipped by the database and not returned.
   */
  claimNotices(limit: number, now: string): Promise<ClaimedNotice[]>;
  /** Record how sending a claimed notice ended. `to` is the address used (for `sent`). */
  finishNotice(id: number, outcome: NoticeOutcome, to: string | null, detail: string, now: string): Promise<void>;
  /**
   * Store the judged commit (a 40-hex sha, or `none`), the time it was decided, and how it was found.
   * The first one stored stays. `detail` left out stores no detail (all null, no fallback).
   */
  recordFinalCommit(memberId: string, runId: string, commit: string, at: string, detail?: FinalCommitDetail): Promise<void>;
  /** A stored chunk of the entry that starts at `offset`, or null. */
  getChunk(memberId: string, runId: string, entryId: string, offset: number): Promise<ChunkRecord | null>;
  /**
   * Advance the chunk's entry past `chunk` and record the chunk, only if the entry still
   * stands where the chunk expects; in the same step add `stored_bytes` and one chunk to the entry's counters.
   */
  commitChunk(args: CommitChunkArgs): Promise<CommitChunkResult>;
  /** Seal the entry. A repeat with the same hashes is `already_sealed`; different hashes once sealed are `conflict`. */
  sealRun(args: SealArgs): Promise<SealResult>;
}
