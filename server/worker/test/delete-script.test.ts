// scripts/delete-test-runs.sh against stub psql and curl on PATH: no network, no database.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

const script = join(import.meta.dirname, "..", "scripts", "delete-test-runs.sh");
const key = "traces/test/r0000000000000001/e1/0.zst";
let tmp = "";

afterEach(() => rmSync(tmp, { recursive: true, force: true }));

// A temp dir with stubs: psql answers one run; curl lists one key in goalcup-traces, logs every
// call, and fails every DELETE.
function setup() {
  tmp = mkdtempSync(join(tmpdir(), "delete-script-"));
  const stub = (name: string, body: string) => {
    writeFileSync(join(tmp, name), `#!/bin/sh\n${body}`);
    chmodSync(join(tmp, name), 0o755);
  };
  stub("psql", `cat >/dev/null\necho '{"runs":["r0000000000000001"],"counts":{}}'\n`);
  stub(
    "curl",
    `echo "$*" >>"${tmp}/curl.log"
case " $* " in *" -X DELETE "*) exit 22 ;; esac
out= url=
while [ $# -gt 0 ]; do
  case $1 in -o) out=$2; shift ;; https://*) url=$1 ;; esac
  shift
done
case $url in
  */goalcup-traces/objects) echo '{"success":true,"result":[{"key":"${key}"}]}' >"$out" ;;
  *) echo '{"success":true,"result":[]}' >"$out" ;;
esac
printf 200
`,
  );
}

function run(...args: string[]) {
  return spawnSync("sh", [script, ...args], {
    cwd: tmp,
    encoding: "utf8",
    env: {
      PATH: `${tmp}:${process.env.PATH}`,
      DATABASE_URL: "x",
      CLOUDFLARE_ACCOUNT_ID: "a",
      CLOUDFLARE_API_TOKEN: "t",
    },
  });
}

test("a failed delete writes only the failed objects and says how to retry them", () => {
  setup();
  const r = run("--older-than", "7 days", "--yes");
  assert.equal(r.status, 1, r.stderr);
  assert.equal(readFileSync(join(tmp, "delete-test-runs.failed"), "utf8"), `goalcup-traces ${key}\n`);
  assert.match(r.stderr, /--objects delete-test-runs\.failed/);
  assert.doesNotMatch(r.stderr, /without --older-than/);
});

test("--objects refuses a key outside the fixed prefixes and deletes nothing", () => {
  setup();
  writeFileSync(join(tmp, "plan"), "goalcup-traces other/key\n");
  const r = run("--objects", "plan", "--yes");
  assert.equal(r.status, 1, r.stderr);
  const log = existsSync(join(tmp, "curl.log")) ? readFileSync(join(tmp, "curl.log"), "utf8") : "";
  assert.doesNotMatch(log, /-X DELETE/);
});
