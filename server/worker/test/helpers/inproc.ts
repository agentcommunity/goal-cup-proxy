// The Worker's own `fetch` run inside the test process, over a fake R2 bucket and a clock the test
// moves. For the rules that depend on time (the start window, the end of a run, the close 24
// hours later), which a real `wrangler dev` could only wait for. Chunk bodies need the Workers
// runtime (`FixedLengthStream`), so a test puts chain state in with `LocalDb.commitChunk` instead.

import { mock } from "node:test";
import type { Env } from "../../src/env.ts";
import worker from "../../src/index.ts";
import { LocalDb } from "../../src/db-local.ts";
import { APPROVAL_KEY, Client } from "./client.ts";
import { FakeBucket, asR2 } from "./fake-r2.ts";
import { TEST_SEED } from "./worker.ts";

export interface InProcess {
  api: Client;
  bucket: FakeBucket;
  /** A LocalDb over the same bucket, to put state in or read it back. */
  db: () => LocalDb;
  /** Move the Worker's clock to this instant (RFC 3339). */
  at: (iso: string) => void;
  /** Move the Worker's clock on by this many seconds. */
  after: (seconds: number) => void;
  /** Hand the clock back. Call it in `after`. */
  stop: () => void;
}

/**
 * `vars` are Worker variables on top of a local-backend test setup. `cf` is what Cloudflare would
 * put on every request (`request.cf`).
 */
export function inProcess(start: string, vars: Record<string, string> = {}, cf: Record<string, unknown> | undefined = undefined): InProcess {
  let now = Date.parse(start);
  const clock = mock.method(Date, "now", () => now);
  const bucket = new FakeBucket();
  const env = {
    TRACES: asR2(bucket),
    EVENT_ID: "cup",
    APPROVAL_URL: "https://approve.test/goalcup",
    PRACTICE_EVENT_ID: "practice",
    PRACTICE_RUN_MINUTES: "10",
    DB_BACKEND: "local",
    GITHUB_CHECK: "off",
    RECEIPT_SIGNING_KEY: TEST_SEED,
    TEST_APPROVAL_KEY: APPROVAL_KEY,
    ...vars,
  } as unknown as Env;
  const api = new Client("http://worker.test", async (input, init) => {
    const request = new Request(input, init);
    if (cf !== undefined) Object.defineProperty(request, "cf", { value: cf });
    return worker.fetch(request, env);
  });
  return {
    api,
    bucket,
    db: () => LocalDb.create(asR2(bucket)),
    at: (iso) => {
      now = Date.parse(iso);
    },
    after: (seconds) => {
      now += seconds * 1000;
    },
    stop: () => clock.mock.restore(),
  };
}
