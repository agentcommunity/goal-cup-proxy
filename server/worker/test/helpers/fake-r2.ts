// Just enough of an R2 bucket for LocalDb: get/put/delete with the conditional puts it uses.

interface Stored {
  text: string;
  etag: string;
}

export class FakeBucket {
  readonly map = new Map<string, Stored>();
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
}

export const asR2 = (b: FakeBucket): R2Bucket => b as unknown as R2Bucket;
