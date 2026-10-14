// Just enough of an R2 bucket for LocalDb: get/put/delete with the conditional puts it uses, and list.

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

  async put(key: string, value: string, opts?: { onlyIf?: { etagMatches?: string; etagDoesNotMatch?: string } }) {
    const cur = this.map.get(key);
    const c = opts?.onlyIf;
    if (c?.etagDoesNotMatch === "*" && cur !== undefined) return null;
    if (c?.etagMatches !== undefined && cur?.etag !== c.etagMatches) return null;
    const etag = `etag-${++this.#n}`;
    this.map.set(key, { text: value, etag });
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
