// Starts `wrangler dev` (local mode: no Cloudflare account, no network) as a child process on a
// free port with its own temporary state, and stops it again. Nothing here logs in or deploys.
//
// `node --test` runs test files as parallel processes and several of them start a Worker, and
// workerd binds a few ports of its own. A port read from listen(0) and then closed can be taken
// by someone else before wrangler binds it. So each chosen port is claimed with a file in the
// temp folder that other test processes respect, and a start that still loses a bind race is
// retried with fresh ports.

import { type ChildProcess, spawn } from "node:child_process";
import { closeSync, mkdtempSync, openSync, rmSync, statSync, unlinkSync } from "node:fs";
import { type Server, createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const projectDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** How many times a start that lost a port race is tried before giving up. */
const START_ATTEMPTS = 5;
/** A claim file older than this was left by a killed run and may be taken over. */
const STALE_CLAIM_MS = 10 * 60_000;

export interface RunningWorker {
  url: string;
  /** Where wrangler persisted R2 for this Worker (deleted on stop). */
  stateDir: string;
  logs: () => string;
  stop: () => Promise<void>;
}

function claimPath(port: number): string {
  return join(tmpdir(), `goalcup-worker-port-${port}`);
}

/** Create the claim file for a port; false when another live claim holds it. */
function tryClaim(port: number): boolean {
  const path = claimPath(port);
  for (let i = 0; i < 2; i++) {
    try {
      closeSync(openSync(path, "wx"));
      return true;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e;
    }
    try {
      if (Date.now() - statSync(path).mtimeMs < STALE_CLAIM_MS) return false;
      unlinkSync(path);
    } catch {
      // The claim went away or changed hands meanwhile: try once more, then skip the port.
    }
  }
  return false;
}

function releaseClaims(ports: number[]): void {
  for (const port of ports) {
    try {
      unlinkSync(claimPath(port));
    } catch {
      // already gone
    }
  }
}

function listenOnAnyPort(): Promise<Server> {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => resolve(s));
  });
}

function closeServer(s: Server): Promise<void> {
  return new Promise((resolve) => s.close(() => resolve()));
}

/**
 * Pick `count` distinct ports the OS offers and claim each one. The listening sockets stay open
 * until all are claimed, so the OS cannot hand the same port out twice within one pick.
 */
async function claimPorts(count: number): Promise<number[]> {
  const held: Server[] = [];
  const claimed: number[] = [];
  try {
    for (let tries = 0; claimed.length < count; tries++) {
      if (tries >= 100) throw new Error("could not claim a free port for wrangler dev");
      const s = await listenOnAnyPort();
      held.push(s);
      const { port } = s.address() as { port: number };
      if (tryClaim(port)) claimed.push(port);
    }
    return claimed;
  } catch (e) {
    releaseClaims(claimed);
    throw e;
  } finally {
    await Promise.all(held.map(closeServer));
  }
}

function isBindCollision(output: string): boolean {
  return output.includes("Address already in use") || output.includes("EADDRINUSE");
}

export async function startWorker(vars: Record<string, string>): Promise<RunningWorker> {
  for (let attempt = 1; ; attempt++) {
    const result = await startOnce(vars);
    if ("worker" in result) return result.worker;
    if (!result.collision || attempt >= START_ATTEMPTS) {
      throw new Error(`wrangler dev exited early:\n${result.output}`);
    }
  }
}

type StartResult = { worker: RunningWorker } | { collision: boolean; output: string };

async function startOnce(vars: Record<string, string>): Promise<StartResult> {
  const [port, inspectorPort] = (await claimPorts(2)) as [number, number];
  let dir: string;
  try {
    dir = mkdtempSync(join(tmpdir(), "goalcup-worker-test-"));
  } catch (e) {
    releaseClaims([port, inspectorPort]);
    throw e;
  }
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
  let stopped = false;
  const stop = async () => {
    if (stopped) return;
    stopped = true;
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
    releaseClaims([port, inspectorPort]);
  };

  // Ready when the Worker answers (any answer: an unknown path is a JSON 404 from our code).
  const deadline = Date.now() + 90_000;
  for (;;) {
    if (exited) {
      // Let the last output arrive before reading it.
      await new Promise((r) => setTimeout(r, 100));
      const text = output.join("");
      await stop();
      return { collision: isBindCollision(text), output: text };
    }
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
  return { worker: { url, stateDir: join(dir, "state"), logs: () => output.join(""), stop } };
}
