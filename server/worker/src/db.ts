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

import type { Ack, StartReceipt } from "./sign.ts";
import { addSeconds, formatTime } from "./util.ts";

/** Twenty-four hours after a run ends the server closes it: no upload is accepted after that. */
export const CLOSE_AFTER_SECONDS = 24 * 3600;

export type RunStatus = "open" | "sealed" | "incomplete" | "abandoned";
export type RunMode = "competition" | "practice";

/** The instant a run is over: `deadline_at`, or earlier if a later start replaced it. (Times compare as text.) */
export function endsAt(run: { deadline_at: string; replaced_at: string | null }): string {
  return run.replaced_at !== null && run.replaced_at < run.deadline_at ? run.replaced_at : run.deadline_at;
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
 * `abandoned` and the words already written stay as they are.
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
  /** Set when a later competition start by the account replaced this run: the run ends then. */
  replaced_at: string | null;
  repo_url: string;
  /** The entry of segment 0, the one `POST /v1/runs` made. */
  first_entry_id: string;
  /** The entry of the highest segment. */
  current_entry_id: string;
  /** When that entry was sealed, or null. */
  current_sealed_at: string | null;
}

/** One segment of a run: its own chain, receipt and seal. Segment 0 is made by `POST /v1/runs`. */
export interface EntryRow {
  entry_id: string;
  run_id: string;
  segment: number;
  // What the open (or continue) call sent; the receipt signs these.
  connector_id: string;
  manifest_sha256: string;
  connector_dir_sha256: string;
  workspace: string;
  session_id: string;
  // Where the upload stands.
  offset: number;
  next_seq: number;
  head: string;
  last_ack: Ack | null;
  /** Set once, by the final heartbeat. */
  final: FinalRow | null;
  // Set once.
  receipt: StartReceipt | null;
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
  ack: Ack;
}

/** The facts about the machine the tracer sends: a flat object of strings, numbers, booleans and nulls. */
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

export type StoreReceiptResult =
  | { kind: "stored"; receipt: StartReceipt }
  | { kind: "existing"; receipt: StartReceipt } // same chain_head as the stored one
  | { kind: "conflict" } // a receipt exists with a different chain_head
  | { kind: "closed" } // the run is abandoned or incomplete, or the entry is sealed
  | { kind: "not_found" };

export type AbandonResult = "abandoned" | "has_chunks" | "closed" | "not_found";

export interface CommitChunkArgs {
  member_id: string;
  run_id: string;
  chunk: ChunkRecord;
}
/**
 * `conflict`: offset, prev or first_seq no longer match the entry (someone advanced it first).
 * `beyond_final`: the chunk goes past the end the final heartbeat committed.
 * `closed`: the run is abandoned or incomplete, or the entry is sealed.
 */
export type CommitChunkResult = "committed" | "conflict" | "beyond_final" | "closed" | "not_found";

export interface HeartbeatArgs {
  member_id: string;
  run_id: string;
  entry_id: string;
  seq: number;
  head: string;
  offset_local: number;
  final: boolean;
  repo_head: string | null; // 40 hex, final only
  at: string;
}
/**
 * `recorded`: an ordinary heartbeat was noted. `final_committed` / `final_existing`: the final
 * heartbeat's (seq, head) is now / already was the committed end. `final_conflict`: a different
 * end is committed, or the chain the server holds already contradicts this one. `closed`: the run
 * is abandoned or incomplete.
 */
export type HeartbeatResult = "recorded" | "final_committed" | "final_existing" | "final_conflict" | "closed" | "not_found";

export interface SealArgs {
  member_id: string;
  run_id: string;
  entry_id: string;
  seal_sha256: string;
  baseline_sha256: string;
  seal_key: string;
  baseline_key: string;
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
export type ApproveDeviceResult = "ok" | "not_found" | "expired" | "already_decided";

/** What a run credential stands for. `run_id` is null until it opens a run. */
export interface Credential {
  member_id: string;
  run_id: string | null;
  mode: RunMode;
  expires_at: string;
}

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
   * run's `official_start`); a practice run replaces nothing.
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
  /** A stored chunk of the entry that starts at `offset`, or null. */
  getChunk(memberId: string, runId: string, entryId: string, offset: number): Promise<ChunkRecord | null>;
  storeReceipt(memberId: string, runId: string, entryId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult>;
  abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult>;
  /** Advance the chunk's entry past `chunk` and record the chunk and its ack, only if the entry still stands where the chunk expects. */
  commitChunk(args: CommitChunkArgs): Promise<CommitChunkResult>;
  /** Seal the entry. A repeat with the same hashes is `already_sealed`; different hashes once sealed are `conflict`. */
  sealRun(args: SealArgs): Promise<SealResult>;
}
