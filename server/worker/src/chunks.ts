// PUT /v1/runs/{id}/chunks/{offset}: store bytes of the chain. docs/UPLOAD-API.md has the contract;
// the order of steps below is SERVER-DESIGN.md section 4, "Acceptance of a chunk".
//
//   1. refuse early, before the body is read (token, offset, size, headers, run state)
//   2. a retry of a chunk already stored: answer with the stored ack, body unread
//   3. offset / prev / first_seq must continue exactly where the run stands (409 otherwise)
//   4. stream the body into R2, create-only, with R2 checking the sha256 (422 if wrong)
//   5. one atomic database step advances offset/head and records chunk + ack
//   6. sign and return the ack

import type { Ctx } from "./context.ts";
import type { ChunkRecord, RunRow } from "./db.ts";
import { ApiError, closed, conflict, json, malformed, notFound, tooLarge } from "./errors.ts";
import { FINAL_RETRY_AFTER, isStopped, requireOpen, requireRun } from "./runs.ts";
import { formatTime, isEntryId, isHex64, parseSafeUint } from "./util.ts";

export const MAX_CHUNK_BYTES = 33_554_432; // 32 MiB, compressed

interface ChunkHeaders {
  entryId: string;
  length: number; // uncompressed bytes the chunk holds
  sha256: string; // of the request body
  firstSeq: number;
  lastSeq: number;
  prev: string;
  head: string;
}

export async function putChunk(ctx: Ctx, member: string, runId: string, offsetText: string, request: Request): Promise<Response> {
  // 1. Everything that needs no database and no body.
  const offset = parseSafeUint(offsetText);
  if (offset === null) throw malformed("The offset in the path must be a non-negative whole number.");
  const contentLength = checkContentLength(request);
  const h = parseHeaders(request);

  // 1. ...and the run: it must be this member's and open (not sealed, not abandoned).
  const run = await requireRun(ctx, member, runId);
  requireOpen(run);
  if (h.entryId !== run.entry_id) throw notFound("That entry does not belong to this run.");

  // 1. ...and the deadline. Past `deadline_at` a chunk is accepted only once the final heartbeat
  //    has committed where the chain ends (until then: wait, 503). Once an end is committed, whether
  //    or not the deadline has passed, nothing may go beyond it (410).
  if (run.final === null && isStopped(ctx, run)) {
    throw new ApiError(503, "awaiting_final", "The run is past its deadline: send the final heartbeat, then upload the rest.", {
      "retry-after": String(FINAL_RETRY_AFTER),
    });
  }
  if (run.final !== null && goesBeyond(h, run.final)) {
    throw closed("This chunk goes past the end of the run that the final heartbeat committed.");
  }

  // 2. A chunk the server already holds, sent again (a reply was lost): same offset, same length,
  //    same head -> the stored ack, without touching the body.
  if (offset < run.offset) {
    const stored = await ctx.db.getChunk(member, runId, offset);
    if (stored !== null && stored.length === h.length && stored.head === h.head) return storedAnswer(stored);
    throw conflict(`The server already holds bytes up to offset ${run.offset} and nothing like this chunk at ${offset}.`);
  }

  // 3. Continue exactly where the server is.
  if (offset !== run.offset) throw conflict(`This chunk starts at ${offset} but the server expects offset ${run.offset}.`);
  if (h.prev !== run.head) throw conflict("Goalcup-Prev is not the head the server holds.");
  if (h.firstSeq !== run.next_seq) throw conflict(`Goalcup-First-Seq is ${h.firstSeq} but the server expects ${run.next_seq}.`);

  // 4. Stream into R2.
  const key = chunkKey(run, offset, h.sha256);
  await storeBody(ctx, request, contentLength, key, h.sha256);

  // 5. + 6. Ack, then one atomic step that records everything.
  const receivedAt = formatTime(ctx.now());
  const ack = await ctx.signer.signAck(run.entry_id, h.lastSeq, h.head, receivedAt);
  const chunk: ChunkRecord = {
    entry_id: run.entry_id,
    offset,
    length: h.length,
    sha256: h.sha256,
    first_seq: h.firstSeq,
    last_seq: h.lastSeq,
    prev: h.prev,
    head: h.head,
    r2_key: key,
    received_at: receivedAt,
    ack,
  };
  const result = await ctx.db.commitChunk({ member_id: member, run_id: runId, chunk });
  switch (result) {
    case "committed":
      return storedAnswer(chunk);
    case "conflict": {
      // Someone moved the run between our check and our commit. If that someone was a retry of
      // this very chunk, its ack is the answer; otherwise the tracer must re-read the position.
      const stored = await ctx.db.getChunk(member, runId, offset);
      if (stored !== null && stored.length === h.length && stored.head === h.head) return storedAnswer(stored);
      throw conflict("The run moved while this chunk was being stored; read the position again.");
    }
    case "beyond_final":
      throw closed("This chunk goes past the end of the run that the final heartbeat committed.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("No such run for this token.");
  }
}

/** Past the committed end: it ends after the final seq, or ends at it with another head. */
function goesBeyond(h: ChunkHeaders, final: { seq: number; head: string }): boolean {
  return h.lastSeq > final.seq || (h.lastSeq === final.seq && h.head !== final.head);
}

function storedAnswer(c: ChunkRecord): Response {
  return json(200, { ack: c.ack, offset: c.offset + c.length, next_seq: c.last_seq + 1, head: c.head });
}

/** traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst. The hash is in the name, so a name is only ever those bytes. */
function chunkKey(run: RunRow, offset: number, sha256: string): string {
  return `traces/${run.event_id}/${run.run_id}/${run.entry_id}/${String(offset).padStart(16, "0")}-${sha256}.zst`;
}

function checkContentLength(request: Request): number {
  const raw = request.headers.get("content-length");
  if (raw === null) throw malformed("Content-Length is required.");
  const n = parseSafeUint(raw);
  if (n === null) throw malformed("Content-Length is not a number.");
  if (n > MAX_CHUNK_BYTES) throw tooLarge(`A chunk is at most ${MAX_CHUNK_BYTES} bytes compressed.`);
  if (n === 0) throw malformed("The body is empty.");
  if (request.body === null) throw malformed("The body is missing.");
  return n;
}

function parseHeaders(request: Request): ChunkHeaders {
  const get = (name: string): string => {
    const v = request.headers.get(name);
    if (v === null) throw malformed(`The ${name} header is required.`);
    return v;
  };
  const type = (request.headers.get("content-type") ?? "").split(";")[0]!.trim().toLowerCase();
  if (type !== "application/zstd") throw malformed("Content-Type must be application/zstd.");

  const entryId = get("Goalcup-Entry-Id");
  if (!isEntryId(entryId)) throw malformed("Goalcup-Entry-Id must be e and 16 lowercase hex digits.");
  const length = parseSafeUint(get("Goalcup-Length"));
  if (length === null || length === 0) throw malformed("Goalcup-Length must be a positive whole number.");
  const sha256 = get("Goalcup-Sha256");
  if (!isHex64(sha256)) throw malformed("Goalcup-Sha256 must be 64 lowercase hex digits.");
  const firstSeq = parseSafeUint(get("Goalcup-First-Seq"));
  const lastSeq = parseSafeUint(get("Goalcup-Last-Seq"));
  if (firstSeq === null || lastSeq === null) throw malformed("Goalcup-First-Seq and Goalcup-Last-Seq must be whole numbers.");
  if (lastSeq < firstSeq) throw malformed("Goalcup-Last-Seq is before Goalcup-First-Seq.");
  const prev = get("Goalcup-Prev");
  const head = get("Goalcup-Head");
  if (!isHex64(prev) || !isHex64(head)) throw malformed("Goalcup-Prev and Goalcup-Head must be 64 lowercase hex digits.");
  return { entryId, length, sha256, firstSeq, lastSeq, prev, head };
}

/**
 * Stream the request body into R2 without holding it in memory. R2 hashes what it receives and
 * refuses the object if the hash is not `sha256` (nothing is stored; 422). The put is
 * create-only: if the key exists it holds bytes that earlier passed this same hash check (the hash
 * is in the key), e.g. a previous attempt stored them but its database step failed, so continuing is safe.
 */
async function storeBody(ctx: Ctx, request: Request, contentLength: number, key: string, sha256: string): Promise<void> {
  // FixedLengthStream gives R2 the length up front and fails the upload if the body is a byte
  // longer or shorter than Content-Length said.
  const { readable, writable } = new FixedLengthStream(contentLength);
  // Not awaited: if R2 finishes without reading (key exists), waiting here would never end.
  // An error in the pipe also fails the put, which is where it is handled.
  request.body!.pipeTo(writable).catch(() => {});
  try {
    await ctx.env.TRACES.put(key, readable, {
      sha256,
      httpMetadata: { contentType: "application/zstd" },
      onlyIf: { etagDoesNotMatch: "*" },
    });
  } catch (e) {
    const text = String(e);
    // Recognising a checksum failure by its text is only proven in the local simulator (R2 error
    // code 10037, "The SHA-256 checksum you specified did not match"). It must be re-proven
    // against real R2 before launch (README, "Before launch"). Anything not recognised is logged
    // under its own name, so a real-R2 wording difference shows up as 502s with this line.
    if (/10037|checksum|did not match/i.test(text)) {
      console.warn(`r2-put-digest-mismatch ${key}: ${text}`);
      throw new ApiError(422, "digest_mismatch", "The body's SHA-256 is not Goalcup-Sha256. Nothing was stored.");
    }
    console.error(`r2-put-unrecognised-error ${key}: ${text}`);
    throw new ApiError(502, "upstream_error", "Storage could not keep the chunk right now.");
  }
}
