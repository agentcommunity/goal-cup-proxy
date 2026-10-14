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
import { addSeconds, formatTime, parseTime } from "./util.ts";

/** A run that is still unsealed this long (24 hours) after its `deadline_at` is abandoned. */
export const ABANDON_AFTER_SECONDS = 24 * 3600;

/** True once `now` is more than `afterSeconds` past `deadlineAt`. (Only tests pass a shorter time.) */
export function isAbandonable(deadlineAt: string, now: Date, afterSeconds: number = ABANDON_AFTER_SECONDS): boolean {
  return now.getTime() > addSeconds(parseTime(deadlineAt), afterSeconds).getTime();
}

/** True once the server's clock is at or past `deadlineAt`: the run is over. */
export function isPastDeadline(deadlineAt: string, now: Date): boolean {
  return now.getTime() >= parseTime(deadlineAt).getTime();
}

/** A run credential that has not opened a run yet lasts this long after the approval. */
export const UNBOUND_CREDENTIAL_SECONDS = 3600;
/** Once it has opened a run, a credential lasts until this long after the run's `deadline_at`. */
export const BOUND_CREDENTIAL_EXTRA_SECONDS = 24 * 3600;

/** The `expires_at` of a credential that has opened a run with this deadline. */
export function boundCredentialExpiry(deadlineAt: string): string {
  return formatTime(addSeconds(parseTime(deadlineAt), BOUND_CREDENTIAL_EXTRA_SECONDS));
}

export type RunStatus = "open" | "sealed" | "abandoned";

/** The first record of a v5 chain (the baseline) has seq 0; a fresh run's next_seq is 0. */
export const FIRST_SEQ = 0;
/** The head before any record: 64 zeros, the chain's genesis. */
export const GENESIS = "0".repeat(64);

/** The end of the run, committed by the final heartbeat: the last record the server will ever accept. */
export interface FinalRow {
  seq: number;
  head: string;
}

export interface RunRow {
  run_id: string;
  entry_id: string;
  event_id: string;
  member_id: string;
  status: RunStatus;
  official_start: string;
  deadline_at: string;
  // What the open call sent; the receipt signs these.
  connector_id: string;
  manifest_sha256: string;
  connector_dir_sha256: string;
  workspace: string;
  session_id: string;
  repo_url: string;
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

export interface OpenRunArgs {
  member_id: string;
  event_id: string;
  run_id: string; // freshly minted; ignored if an existing run is returned
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
}

export type OpenRunResult =
  | { kind: "created"; run: RunRow }
  | { kind: "existing"; run: RunRow } // same session_id, open, no receipt yet
  | { kind: "conflict" } // the account has another open run
  | { kind: "forbidden"; message: string }; // terms not accepted, or outside the start window

export type StoreReceiptResult =
  | { kind: "stored"; receipt: StartReceipt }
  | { kind: "existing"; receipt: StartReceipt } // same chain_head as the stored one
  | { kind: "conflict" } // a receipt exists with a different chain_head
  | { kind: "closed" };

export type AbandonResult = "abandoned" | "has_chunks" | "closed" | "not_found";

export interface CommitChunkArgs {
  member_id: string;
  run_id: string;
  chunk: ChunkRecord;
}
/**
 * `conflict`: offset, prev or first_seq no longer match the run (someone advanced it first).
 * `beyond_final`: the chunk goes past the end the final heartbeat committed.
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
 * end is committed, or the chain the server holds already contradicts this one. `closed`: abandoned.
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
export type SealResult = "sealed" | "already_sealed" | "conflict" | "closed" | "not_found";

export type DevicePurpose = "login" | "run";
export type RunMode = "competition" | "practice";

/** `conflict`: the credential is unknown, or already bound to another run. */
export type BindCredentialResult = "bound" | "conflict";

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
   * The credential opened this run: from now on it sees only this run, until `expiresAt`. Binds
   * only a credential that is unbound or already bound to this run; anything else is `conflict`.
   */
  bindCredential(credentialSha256: string, runId: string, expiresAt: string): Promise<BindCredentialResult>;
  openRun(args: OpenRunArgs): Promise<OpenRunResult>;
  getRun(memberId: string, runId: string): Promise<RunRow | null>;
  /** Note a heartbeat; a final one commits the end of the run (once). */
  recordHeartbeat(args: HeartbeatArgs): Promise<HeartbeatResult>;
  /** Abandon an open, unsealed run whose time ran out; true if it was abandoned now. */
  expireRun(memberId: string, runId: string, at: string): Promise<boolean>;
  /** A stored chunk that starts at `offset`, or null. */
  getChunk(memberId: string, runId: string, offset: number): Promise<ChunkRecord | null>;
  storeReceipt(memberId: string, runId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult>;
  abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult>;
  /** Advance the run past `chunk` and record the chunk and its ack, only if the run still stands where the chunk expects. */
  commitChunk(args: CommitChunkArgs): Promise<CommitChunkResult>;
  /** Mark sealed. A repeat with the same hashes is `already_sealed`; different hashes once sealed are `conflict`. */
  sealRun(args: SealArgs): Promise<SealResult>;
}
