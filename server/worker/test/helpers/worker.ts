// Starts `wrangler dev` (local mode: no Cloudflare account, no network) as a child process on a
// free port with its own temporary state, and stops it again. Nothing here logs in or deploys.
//
// `node --test` runs test files as parallel processes and several of them start a Worker, and
// workerd binds a few ports of its own. A port read from listen(0) and then closed can be taken
// by someone else before wrangler binds it, so a start that loses that race is retried with
// fresh ports and a fresh state dir.

import { type ChildProcess, spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { type Server, createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const projectDir = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** How many times a start that lost a port race is tried before giving up. */
const START_ATTEMPTS = 5;
/** How long to wait for wrangler's output pipes to end after it exits. */
const CLOSE_WAIT_MS = 5000;

export interface RunningWorker {
  url: string;
  /** Where wrangler persisted R2 for this Worker (deleted on stop). */
  stateDir: string;
  logs: () => string;
  stop: () => Promise<void>;
}

function listenOnAnyPort(): Promise<Server> {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.once("error", reject);
    s.listen(0, "127.0.0.1", () => resolve(s));
  });
}

function portOf(s: Server): number {
  return (s.address() as { port: number }).port;
}

function closeServer(s: Server): Promise<void> {
  return new Promise((resolve) => s.close(() => resolve()));
}

/**
 * Two distinct ports the OS offers. Both sockets stay open until both are read, so one start
 * never gets the same port twice.
 */
async function pickTwoPorts(): Promise<[number, number]> {
  const a = await listenOnAnyPort();
  try {
    const b = await listenOnAnyPort();
    const ports: [number, number] = [portOf(a), portOf(b)];
    await closeServer(b);
    return ports;
  } finally {
    await closeServer(a);
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
  const [port, inspectorPort] = await pickTwoPorts();
  const dir = mkdtempSync(join(tmpdir(), "goalcup-worker-test-"));
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
  // "close" fires once the child's stdout and stderr have ended, so all its output is in.
  const closed = new Promise<void>((resolve) => child.once("close", () => resolve()));

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
    if (exited) {
      // Read the output only after the pipes have ended. The bound covers a grandchild that
      // might still hold them; it is not a guess at how long the last bytes take.
      await Promise.race([closed, new Promise((r) => setTimeout(r, CLOSE_WAIT_MS).unref())]);
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
