// Just enough of an R2 bucket for LocalDb: get/put/delete with the conditional puts it uses, and list.

import { createHash } from "node:crypto";

interface Stored {
  text: string;
  etag: string;
}

export class FakeBucket {
  readonly map = new Map<string, Stored>();
  /** The most keys one `list` answers with (R2's is 1000); a test lowers it to make a listing take several pages. */
  pageLimit = 1000;
  #n = 0;

  async get(key: string) {
    const o = this.map.get(key);
    return o === undefined ? null : { etag: o.etag, json: async () => JSON.parse(o.text) as unknown };
  }

  /**
   * A string (LocalDb's state), or a stream (a chunk body, as the Worker's upload sends it): read in
   * full, and refused like R2 refuses it when `sha256` is given and does not match.
   */
  async put(key: string, value: string | ReadableStream<Uint8Array>, opts?: { onlyIf?: { etagMatches?: string; etagDoesNotMatch?: string }; sha256?: string }) {
    const cur = this.map.get(key);
    const c = opts?.onlyIf;
    if (c?.etagDoesNotMatch === "*" && cur !== undefined) return null;
    if (c?.etagMatches !== undefined && cur?.etag !== c.etagMatches) return null;
    let text: string;
    if (typeof value === "string") text = value;
    else {
      const bytes = new Uint8Array(await new Response(value).arrayBuffer());
      if (opts?.sha256 !== undefined && createHash("sha256").update(bytes).digest("hex") !== opts.sha256) {
        throw new Error("put: (10037) The SHA-256 checksum you specified did not match what we received.");
      }
      text = Buffer.from(bytes).toString("base64");
    }
    const etag = `etag-${++this.#n}`;
    this.map.set(key, { text, etag });
    return { etag };
  }

  async delete(key: string) {
    this.map.delete(key);
  }

  /** Keys under `prefix`, in key order, `limit` at a time; `cursor` is the last key of the page before. */
  async list(opts: { prefix?: string; cursor?: string; limit?: number } = {}) {
    const keys = [...this.map.keys()].filter((k) => k.startsWith(opts.prefix ?? "") && (opts.cursor === undefined || k > opts.cursor)).sort();
    const page = keys.slice(0, Math.min(opts.limit ?? 1000, this.pageLimit));
    const truncated = keys.length > page.length;
    return { objects: page.map((key) => ({ key })), truncated, cursor: truncated ? page[page.length - 1] : undefined };
  }
}

export const asR2 = (b: FakeBucket): R2Bucket => b as unknown as R2Bucket;
