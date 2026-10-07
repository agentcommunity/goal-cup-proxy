/**
 * goal-cup capture spike 5 - OpenCode folder-scoped tracker shim.
 *
 * Placement: <workspace>/.opencode/plugin/goalcup-tracker.ts
 * Loading  : auto-discovered, NO config entry and NO global config needed.
 *            opencode 1.18.27 bundled skill doc (`customize-opencode`), verbatim:
 *              "Auto-discovered plugins (no config entry needed): any `*.ts` or `*.js`
 *               file in `.opencode/plugin/` or `.opencode/plugins/`."
 *            Loader glob in the binary: `{plugin,plugins}/*.{ts,js}`.
 *
 * Contract: record raw, interpret later (AGENTS.md). Every hook payload is forwarded
 * verbatim to the stub receiver as JSON. The only shaping done here is:
 *   - `event` bus payloads are BATCHED (see FLUSH_MS) because the `event` hook is called
 *     fire-and-forget from an Effect.sync loop and `message.part.delta` fires per token;
 *     one HTTP connection per delta would overrun the single-threaded stub receiver.
 *     Every event keeps its own seq and timestamp inside the batch. Nothing is dropped
 *     or rewritten. The SSE leg (GET /event) is the unbatched second copy.
 *   - two payloads are reduced to KEY NAMES ONLY because their values are credentials:
 *       `chat.headers` output.headers  (provider Authorization headers)
 *       `shell.env`    output env      (the whole process environment)
 *     Everything else, including the full system prompt array and the full message
 *     history, is posted whole.
 *
 * Blocking (assertion A7): opencode has no prompt-submit hook with a decision protocol.
 * `chat.message` is the only hook that runs BEFORE the user message is persisted, and
 * `Plugin.trigger` awaits it via `Effect.promise(async () => hook(input, output))`, which
 * turns a rejection into an Effect defect that `Plugin.trigger` does not catch. So the
 * only way to refuse a message is to throw, which aborts the turn rather than returning a
 * clean rejection. This shim forwards the receiver's `{"decision":"block"}` by throwing.
 * Set GOALCUP_BLOCK_MODE=off to record the violation without throwing.
 *
 * Env (all process-scoped, set by run.sh):
 *   GOALCUP_RECV              receiver base URL   (default http://127.0.0.1:7777)
 *   GOALCUP_RUN               run id, echoed in every post
 *   GOALCUP_BLOCK_MODE        throw | off        (default throw)
 *   GOALCUP_RECV_TIMEOUT_MS   per-post timeout   (default 5000)
 */

const RECV = process.env["GOALCUP_RECV"] || "http://127.0.0.1:7777"
const RUN = process.env["GOALCUP_RUN"] || "unknown-run"
const BLOCK_MODE = process.env["GOALCUP_BLOCK_MODE"] || "throw"
const TIMEOUT_MS = Number(process.env["GOALCUP_RECV_TIMEOUT_MS"] || 5000)
const FLUSH_MS = Number(process.env["GOALCUP_EVENT_FLUSH_MS"] || 150)
const FLUSH_MAX = Number(process.env["GOALCUP_EVENT_FLUSH_MAX"] || 64)

const SHIM = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
let SEQ = 0

function now(): number {
  return Date.now() / 1000
}

function encode(obj: unknown): string {
  try {
    return JSON.stringify(obj)
  } catch (e) {
    return JSON.stringify({ _unserializable: true, error: String(e) })
  }
}

async function post(hook: string, payload: Record<string, unknown>): Promise<any> {
  const n = ++SEQ
  const body = encode({ shim: SHIM, run: RUN, seq: n, hook, t: now(), ...payload })
  try {
    const res = await fetch(`${RECV}/${hook}`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-GoalCup-Hook": hook,
        "X-GoalCup-Shim": SHIM,
        "X-GoalCup-Seq": String(n),
        "X-GoalCup-Run": RUN,
      },
      body,
      signal: AbortSignal.timeout(TIMEOUT_MS),
    })
    const txt = await res.text()
    try {
      return JSON.parse(txt)
    } catch {
      return {}
    }
  } catch (e) {
    try {
      console.error("[goalcup] post failed", hook, String(e))
    } catch {}
    return {}
  }
}

// ---------------------------------------------------------------- event batching
const queue: unknown[] = []
let timer: any = null

async function flush(): Promise<void> {
  if (timer) {
    clearTimeout(timer)
    timer = null
  }
  if (queue.length === 0) return
  const batch = queue.splice(0, queue.length)
  await post("event", { batch, count: batch.length })
}

function enqueue(item: unknown): void {
  queue.push(item)
  if (queue.length >= FLUSH_MAX) {
    void flush()
    return
  }
  if (!timer) timer = setTimeout(() => void flush(), FLUSH_MS)
}

function names(o: unknown): string[] {
  if (!o || typeof o !== "object") return []
  return Object.keys(o as Record<string, unknown>)
}

// ---------------------------------------------------------------- plugin
export default async (input: any) => {
  await post("plugin.loaded", {
    pluginInput: {
      directory: input?.directory ?? null,
      worktree: input?.worktree ?? null,
      serverUrl: input?.serverUrl ? String(input.serverUrl) : null,
      project: input?.project ?? null,
    },
    process: {
      pid: process.pid,
      cwd: process.cwd(),
      argv: process.argv,
      opencodeClient: process.env["OPENCODE_CLIENT"] ?? null,
      xdgDataHome: process.env["XDG_DATA_HOME"] ?? null,
      xdgConfigHome: process.env["XDG_CONFIG_HOME"] ?? null,
      xdgStateHome: process.env["XDG_STATE_HOME"] ?? null,
      xdgCacheHome: process.env["XDG_CACHE_HOME"] ?? null,
    },
  })

  return {
    // every bus event, batched, raw
    event: (arg: any) => {
      enqueue({ seq: ++SEQ, t: now(), event: arg?.event ?? arg })
    },

    // merged config, once on init. Keys + the fields we care about; the full config can
    // carry provider credentials via {env:...} resolution, so it is not posted whole.
    config: async (cfg: any) => {
      await post("config", {
        keys: names(cfg),
        plugin: cfg?.plugin ?? null,
        permission: cfg?.permission ?? null,
        model: cfg?.model ?? null,
        share: cfg?.share ?? null,
        experimental: cfg?.experimental ?? null,
      })
    },

    // fires BEFORE the user message is persisted -> the only blocking surface
    "chat.message": async (i: any, o: any) => {
      const parts = Array.isArray(o?.parts) ? o.parts : []
      const prompt = parts
        .filter((p: any) => p?.type === "text")
        .map((p: any) => p?.text ?? "")
        .join("")
      await flush()
      const r = await post("chat.message", {
        input: i,
        output: o,
        // top-level `prompt` so spikes/stub-receiver.py --block-over N can decide
        prompt,
        promptLength: prompt.length,
      })
      if (r && r.decision === "block") {
        await post("chat.message.blocked", {
          sessionID: i?.sessionID ?? null,
          messageID: i?.messageID ?? null,
          promptLength: prompt.length,
          reason: r.reason ?? "",
          enforced: BLOCK_MODE !== "off",
        })
        if (BLOCK_MODE !== "off") {
          throw new Error(`[goal-cup] ${r.reason || "message rejected by tracker"}`)
        }
      }
    },

    "chat.params": async (i: any, o: any) => {
      await post("chat.params", { input: i, output: o })
    },

    // values are provider credentials -> names only
    "chat.headers": async (i: any, o: any) => {
      await post("chat.headers", {
        sessionID: i?.sessionID ?? null,
        agent: i?.agent ?? null,
        model: i?.model ?? null,
        provider: i?.provider ?? null,
        headerNames: names(o?.headers),
      })
    },

    "tool.execute.before": async (i: any, o: any) => {
      await post("tool.execute.before", { input: i, output: o })
    },

    // post-truncation in 1.18.27: tools truncate inside execute(), and the MCP tools
    // build `{output: truncated.content, metadata: {truncated, outputPath}}` BEFORE the
    // trigger. metadata.outputPath is the spill file under $data/tool-output/.
    "tool.execute.after": async (i: any, o: any) => {
      await post("tool.execute.after", { input: i, output: o })
    },

    "tool.definition": async (i: any, o: any) => {
      await post("tool.definition", { input: i, outputKeys: names(o) })
    },

    "command.execute.before": async (i: any, o: any) => {
      await post("command.execute.before", { input: i, output: o })
    },

    "permission.ask": async (p: any, o: any) => {
      await post("permission.ask", { permission: p, output: o })
    },

    // the whole environment -> names only
    "shell.env": async (i: any, o: any) => {
      await post("shell.env", { input: i, envNames: names(o?.env ?? o) })
    },

    // the exact system prompt array sent to the model, whole
    "experimental.chat.system.transform": async (i: any, o: any) => {
      const sys = Array.isArray(o?.system) ? o.system : []
      await post("experimental.chat.system.transform", {
        input: i,
        system: sys,
        systemCount: sys.length,
        systemLengths: sys.map((s: any) => String(s ?? "").length),
      })
    },

    // the exact message history sent to the model, whole
    "experimental.chat.messages.transform": async (i: any, o: any) => {
      const msgs = Array.isArray(o?.messages) ? o.messages : []
      await post("experimental.chat.messages.transform", {
        input: i,
        messages: msgs,
        messageCount: msgs.length,
      })
    },

    "experimental.session.compacting": async (i: any, o: any) => {
      await post("experimental.session.compacting", { input: i, output: o })
    },

    "experimental.compaction.autocontinue": async (i: any, o: any) => {
      await post("experimental.compaction.autocontinue", { input: i, output: o })
    },

    "experimental.text.complete": async (i: any, o: any) => {
      await post("experimental.text.complete", { input: i, output: o })
    },

    dispose: async () => {
      await flush()
      await post("dispose", {})
    },
  }
}
