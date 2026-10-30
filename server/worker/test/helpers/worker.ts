// Starts `wrangler dev` (local mode: no Cloudflare account, no network) as a child process on a
// free port with its own temporary state, and stops it again. Nothing here logs in or deploys.

import { type ChildProcess, spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const projectDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

export interface RunningWorker {
  url: string;
  /** Where wrangler persisted R2 for this Worker (deleted on stop). */
  stateDir: string;
  logs: () => string;
  stop: () => Promise<void>;
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address() as { port: number };
      s.close(() => resolve(port));
    });
  });
}

export async function startWorker(vars: Record<string, string>): Promise<RunningWorker> {
  const dir = mkdtempSync(join(tmpdir(), "goalcup-worker-test-"));
  const port = await freePort();
  const inspectorPort = await freePort();
  const allVars: Record<string, string> = {
    DB_BACKEND: "local",
    GITHUB_CHECK: "off",
    ...vars,
  };
  const args = [
    join(projectDir, "node_modules", "wrangler", "bin", "wrangler.js"),
    "dev",
    "--ip", "127.0.0.1",
    "--port", String(port),
    "--inspector-port", String(inspectorPort),
    "--persist-to", join(dir, "state"),
    "--show-interactive-dev-session=false",
    ...Object.entries(allVars).flatMap(([k, v]) => ["--var", `${k}:${v}`]),
  ];
  const output: string[] = [];
  const child: ChildProcess = spawn(process.execPath, args, {
    cwd: projectDir,
    env: {
      ...process.env,
      // Keep wrangler's own files and any prompts out of the user's real config.
      XDG_CONFIG_HOME: join(dir, "config"),
      WRANGLER_LOG_PATH: join(dir, "logs"),
      WRANGLER_REGISTRY_PATH: join(dir, "registry"),
      WRANGLER_SEND_METRICS: "false",
      CLOUDFLARE_API_TOKEN: "",
      CLOUDFLARE_ACCOUNT_ID: "",
      CI: "1",
      NO_COLOR: "1",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout?.on("data", (d) => output.push(String(d)));
  child.stderr?.on("data", (d) => output.push(String(d)));
  let exited = false;
  child.once("exit", () => {
    exited = true;
  });

  const url = `http://127.0.0.1:${port}`;
  const stop = async () => {
    if (!exited) {
      child.kill("SIGTERM");
      await new Promise<void>((resolve) => {
        const t = setTimeout(() => {
          child.kill("SIGKILL");
          resolve();
        }, 5000);
        child.once("exit", () => {
          clearTimeout(t);
          resolve();
        });
      });
    }
    rmSync(dir, { recursive: true, force: true });
  };

  // Ready when the Worker answers (any answer: an unknown path is a JSON 404 from our code).
  const deadline = Date.now() + 90_000;
  for (;;) {
    if (exited) throw new Error(`wrangler dev exited early:\n${output.join("")}`);
    try {
      const res = await fetch(`${url}/v1/ready-check`, { signal: AbortSignal.timeout(2000) });
      if (res.status === 404 && (await res.text()).includes("not_found")) break;
    } catch {
      // not up yet
    }
    if (Date.now() > deadline) {
      await stop();
      throw new Error(`wrangler dev did not become ready:\n${output.join("")}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  return { url, stateDir: join(dir, "state"), logs: () => output.join(""), stop };
}
