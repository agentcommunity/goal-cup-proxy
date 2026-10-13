// The Worker's view of its database: one method per step of an API call. Two implementations
// (db-supabase.ts for production, db-local.ts for wrangler dev and tests) must behave the same;
// test/integration.test.ts exercises the local one, and the SQL in sql/ mirrors it.
//
// Rules every implementation keeps:
//   * A run belongs to one member. Asking for another member's run gives null ("not found"),
//     never a different answer, so a token cannot probe for other members' runs.
//   * Every write is atomic: it either applies fully against the state the caller saw, or it
//     reports why not and changes nothing.

import type { Ack, StartReceipt } from "./sign.ts";

export type RunStatus = "open" | "sealed" | "abandoned";

/** The first record of a v5 chain (the baseline) has seq 0; a fresh run's next_seq is 0. */
export const FIRST_SEQ = 0;
/** The head before any record: 64 zeros, the chain's genesis. */
export const GENESIS = "0".repeat(64);

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
/** `conflict`: offset, prev or first_seq no longer match the run (someone advanced it first). */
export type CommitChunkResult = "committed" | "conflict" | "closed" | "not_found";

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

export interface Db {
  /** sha256 hex of the bearer token -> member id, or null if unknown or revoked. */
  memberForToken(tokenSha256: string): Promise<string | null>;
  openRun(args: OpenRunArgs): Promise<OpenRunResult>;
  getRun(memberId: string, runId: string): Promise<RunRow | null>;
  /** A stored chunk that starts at `offset`, or null. */
  getChunk(memberId: string, runId: string, offset: number): Promise<ChunkRecord | null>;
  storeReceipt(memberId: string, runId: string, chainHead: string, receipt: StartReceipt): Promise<StoreReceiptResult>;
  abandonRun(memberId: string, runId: string, reason: string, at: string): Promise<AbandonResult>;
  /** Advance the run past `chunk` and record the chunk and its ack, only if the run still stands where the chunk expects. */
  commitChunk(args: CommitChunkArgs): Promise<CommitChunkResult>;
  /** Mark sealed. A repeat with the same hashes is `already_sealed`; different hashes once sealed are `conflict`. */
  sealRun(args: SealArgs): Promise<SealResult>;
}
