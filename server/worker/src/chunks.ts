// PUT /v1/runs/{id}/chunks/{offset}: store bytes of the chain. docs/UPLOAD-API.md has the contract
// (section "PUT /v1/runs/{run_id}/chunks/{offset}"); the steps below follow its order.
//
//   1. refuse early, before the body is read (token, offset, size, headers, run state)
//   2. a retry of a chunk already stored: answer with the stored ack, body unread (before the
//      run state is looked at, so a retry is answered whatever has happened since)
//   3. offset / prev / first_seq must continue exactly where the entry stands (409 otherwise)
//   3b. the run's size limit: the run's stored bytes plus this chunk's Content-Length must stay
//      within RUN_MAX_COMPRESSED_BYTES (413 quota_exceeded otherwise, body unread, nothing stored)
//   4. stream the body into R2, create-only, with R2 checking the sha256 (422 if wrong)
//   5. one atomic database step advances offset/head, records chunk + ack, and adds the chunk's
//      compressed size to the entry's stored_bytes and one to its chunk_count
//   6. sign and return the ack

import type { Ctx } from "./context.ts";
import { type ChunkRecord, type EntryRow, type RunRow, isPastEnd } from "./db.ts";
import { ApiError, closed, conflict, json, malformed, notFound, tooLarge } from "./errors.ts";
import { FINAL_RETRY_AFTER, requireOpen, requireRun } from "./runs.ts";
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

  // 1. ...and the run and entry: this member's, and the entry one of the run's segments.
  const { run, entry } = await requireRun(ctx, member, runId, h.entryId);

  // 2. A chunk the server already holds, sent again (a reply was lost): same offset, same length,
  //    same head -> the stored ack, without touching the body.
  const held = offset < entry.offset ? await ctx.db.getChunk(member, runId, entry.entry_id, offset) : null;
  if (held !== null && held.length === h.length && held.head === h.head) return storedAnswer(held);

  // 1. ...and the run state: open (not abandoned, not closed) and the entry not sealed.
  requireOpen(ctx, run, entry);

  // 1. ...and the end. Once the run is over a chunk is accepted only after the final heartbeat has
  //    committed where the entry's chain ends (until then: wait, 503). Once an end is committed,
  //    whether or not the run is over, nothing may go beyond it (410).
  if (entry.final === null && isPastEnd(run, ctx.now())) {
    throw new ApiError(503, "awaiting_final", "The run is over: send the final heartbeat, then upload the rest.", {
      "retry-after": String(FINAL_RETRY_AFTER),
    });
  }
  if (entry.final !== null && goesBeyond(h, entry.final)) {
    throw closed("This chunk goes past the end of the entry that the final heartbeat committed.");
  }
  if (offset < entry.offset) {
    throw conflict(`The server already holds bytes up to offset ${entry.offset} and nothing like this chunk at ${offset}.`);
  }

  // 3. Continue exactly where the server is.
  if (offset !== entry.offset) throw conflict(`This chunk starts at ${offset} but the server expects offset ${entry.offset}.`);
  if (h.prev !== entry.head) throw conflict("Goalcup-Prev is not the head the server holds.");
  if (h.firstSeq !== entry.next_seq) throw conflict(`Goalcup-First-Seq is ${h.firstSeq} but the server expects ${entry.next_seq}.`);

  // 3b. The run's size limit, before a byte of the body is read or stored. A retry of a chunk the
  //     server holds was answered above, so it is never refused here.
  checkQuota(ctx, run, contentLength);

  // 4. Stream into R2.
  const key = chunkKey(run, entry, offset, h.sha256);
  await storeBody(ctx, request, contentLength, key, h.sha256);

  // 5. + 6. Ack, then one atomic step that records everything.
  const receivedAt = formatTime(ctx.now());
  const ack = await ctx.signer.signAck(entry.entry_id, h.lastSeq, h.head, receivedAt);
  const chunk: ChunkRecord = {
    entry_id: entry.entry_id,
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
  const result = await ctx.db.commitChunk({ member_id: member, run_id: runId, chunk, stored_bytes: contentLength });
  switch (result) {
    case "committed":
      noteQuota(ctx, run, contentLength);
      return storedAnswer(chunk);
    case "conflict": {
      // Someone moved the run between our check and our commit. If that someone was a retry of
      // this very chunk, its ack is the answer; otherwise goalcup must re-read the position.
      const stored = await ctx.db.getChunk(member, runId, entry.entry_id, offset);
      if (stored !== null && stored.length === h.length && stored.head === h.head) return storedAnswer(stored);
      throw conflict("The run moved while this chunk was being stored; read the position again.");
    }
    case "beyond_final":
      throw closed("This chunk goes past the end of the entry that the final heartbeat committed.");
    case "closed":
      throw closed("This run is no longer open.");
    case "not_found":
      throw notFound("That entry does not belong to this run.");
  }
}

/**
 * 413 quota_exceeded when this chunk would take the run past RUN_MAX_COMPRESSED_BYTES. goalcup
 * keeps its data and retries, so raising the variable resumes the upload where it stopped.
 * The check reads the run's total as it was a moment ago, so concurrent uploads to several entries
 * of one run can pass it by at most one chunk (32 MiB) each.
 */
function checkQuota(ctx: Ctx, run: RunRow, contentLength: number): void {
  if (run.stored_bytes === undefined) {
    // The database does not count stored bytes yet (website migration 20261021100000 missing): do
    // not let a run past a limit nobody can see. 502 is retried; goalcup keeps the data.
    console.error(`quota-unknown ${run.run_id}: the database sends no stored_bytes; apply the size-limit migration`);
    throw new ApiError(502, "upstream_error", "The database cannot say how much this run holds right now.");
  }
  const max = ctx.cfg.runMaxCompressedBytes;
  if (run.stored_bytes + contentLength <= max) return;
  // One line per refusal for the operators (Workers Logs). goalcup retries with backoff, so a
  // run at its limit repeats this line every few minutes until the limit is raised or the run ends.
  console.error(`quota-exceeded ${run.run_id} stored=${run.stored_bytes} chunk=${contentLength} max=${max}`);
  throw new ApiError(
    413,
    "quota_exceeded",
    `This run holds ${run.stored_bytes} compressed bytes; this chunk would pass the server's limit of ${max} per run. ` +
      "Nothing was stored. Keep the data: the upload resumes when the organizers raise the limit.",
  );
}

/** A log line for the operators when a committed chunk takes the run to half its limit, or to all of it. */
function noteQuota(ctx: Ctx, run: RunRow, contentLength: number): void {
  if (run.stored_bytes === undefined) return;
  const max = ctx.cfg.runMaxCompressedBytes;
  const before = run.stored_bytes;
  const after = before + contentLength;
  if (before < max && after >= max) console.warn(`quota-full ${run.run_id} stored=${after} max=${max}`);
  else if (2 * before < max && 2 * after >= max) console.warn(`quota-half ${run.run_id} stored=${after} max=${max}`);
}

/** Past the committed end: it ends after the final seq, or ends at it with another head. */
function goesBeyond(h: ChunkHeaders, final: { seq: number; head: string }): boolean {
  return h.lastSeq > final.seq || (h.lastSeq === final.seq && h.head !== final.head);
}

function storedAnswer(c: ChunkRecord): Response {
  return json(200, { ack: c.ack, offset: c.offset + c.length, next_seq: c.last_seq + 1, head: c.head });
}

/** traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst. The hash is in the name, so a name is only ever those bytes. */
function chunkKey(run: RunRow, entry: EntryRow, offset: number, sha256: string): string {
  return `traces/${run.event_id}/${run.run_id}/${entry.entry_id}/${String(offset).padStart(16, "0")}-${sha256}.zst`;
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
