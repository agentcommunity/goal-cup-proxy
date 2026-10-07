// goal-cup capture spike 8 -- OpenClaw plugin shim (folder-scoped).
//
// Lives at <workspace>/.openclaw/extensions/goalcup-capture/ so it is discovered by
// OpenClaw's WORKSPACE plugin root (src/plugins/roots.ts:
//   workspace = path.join(workspaceRoot, ".openclaw", "extensions")).
// Discovery alone is not enough; the isolated openclaw.json must also carry
//   plugins.entries."goalcup-capture".enabled                       = true
//   plugins.entries."goalcup-capture".hooks.allowConversationAccess = true
// (the second one is what non-bundled plugins need for llm_input / llm_output /
// before_agent_run / agent_end -- see docs/plugins/hooks.md "Permissions and scope").
// run.sh additionally pins plugins.load.paths at this directory so the copy that loads is
// unambiguous ("config-selected" is the highest discovery precedence).
//
// No SDK import on purpose: the loader accepts any module whose default export is an
// object with a `register` function (dist/loader-DhyKX__3.js resolvePluginModuleExport),
// so the shim does not have to resolve `openclaw/plugin-sdk/*` from outside the install.
//
// Everything is posted RAW: the whole typed-hook event object plus the whole hook context,
// JSON.stringify'd with no field selection and no truncation. Interpretation is check.py's
// job (AGENTS.md: record raw, interpret later).
//
// Env pins (set by run.sh on the gateway process):
//   GOALCUP_RECV        receiver base url            default http://127.0.0.1:7777
//   GOALCUP_RUN         run id, echoed in every post  default "unset"
//   GOALCUP_TICK_MS     liveness heartbeat period     default 5000, 0 disables
//   GOALCUP_POST_MS     per-post timeout              default 4000
//   GOALCUP_GATE        "on" (default) honours the receiver's block decision on
//                       before_agent_run; "off" records the decision but always passes.

const RECV = (process.env.GOALCUP_RECV || "http://127.0.0.1:7777").replace(/\/+$/, "");
const RUN = process.env.GOALCUP_RUN || "unset";
const TICK_MS = Number(process.env.GOALCUP_TICK_MS ?? 5000);
const POST_MS = Number(process.env.GOALCUP_POST_MS ?? 4000);
const GATE = (process.env.GOALCUP_GATE || "on") !== "off";

let seq = 0;

/** JSON that never throws: cycles and non-serialisable values become markers. */
function safeJson(value) {
  const seen = new WeakSet();
  return JSON.stringify(value, (_k, v) => {
    if (typeof v === "bigint") return { _bigint: v.toString() };
    if (typeof v === "function") return { _function: v.name || "anonymous" };
    if (v instanceof Error) return { _error: v.message, stack: v.stack };
    if (v && typeof v === "object") {
      if (seen.has(v)) return { _cycle: true };
      seen.add(v);
    }
    return v;
  });
}

/**
 * POST one hook payload. Resolves to the receiver's parsed JSON reply, or null.
 * Never throws: a dead receiver must not break the agent run (it must, however, be
 * visible -- a missing post IS the evidence for the hot-reload probe).
 */
async function post(hook, body) {
  seq += 1;
  const envelope = {
    _shim: "goalcup-capture",
    _run: RUN,
    _seq: seq,
    _pid: process.pid,
    _ts: new Date().toISOString(),
    hook,
    ...body,
  };
  let payload;
  try {
    payload = safeJson(envelope);
  } catch (err) {
    payload = safeJson({ _shim: "goalcup-capture", _run: RUN, _seq: seq, hook, _serialise_failed: String(err) });
  }
  try {
    const res = await fetch(`${RECV}/${hook}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-goalcup-run": RUN, "x-goalcup-hook": hook },
      body: payload,
      signal: AbortSignal.timeout(POST_MS),
    });
    const text = await res.text();
    try {
      return JSON.parse(text);
    } catch {
      return null;
    }
  } catch {
    return null;
  }
}

/** Fire-and-forget wrapper for the non-gating hooks. */
function tap(hook, pick) {
  return async (event, ctx) => {
    await post(hook, { ctx, event: pick ? pick(event) : event });
  };
}

export default {
  id: "goalcup-capture",
  name: "Goal Cup capture",
  description: "Spike-only capture shim: posts raw plugin-hook payloads to the tracker receiver.",
  configSchema: {
    // Same contract as the SDK's emptyPluginConfigSchema: accept undefined or {}.
    safeParse(value) {
      if (value === undefined) return { success: true, data: undefined };
      if (!value || typeof value !== "object" || Array.isArray(value)) {
        return { success: false, error: { issues: [{ path: [], message: "expected config object" }] } };
      }
      return { success: true, data: value };
    },
    jsonSchema: { type: "object", additionalProperties: false, properties: {} },
  },

  register(api) {
    // ---- liveness -------------------------------------------------------------
    // Posted synchronously at register() time. Two of these in hooks.jsonl == the
    // gateway restarted or reload-plugins re-imported the module.
    void post("plugin.loaded", {
      env: {
        stateDir: process.env.OPENCLAW_STATE_DIR || null,
        configPath: process.env.OPENCLAW_CONFIG_PATH || null,
        cwd: process.cwd(),
        node: process.version,
        argv: process.argv.slice(1),
      },
    });

    // Independent of any agent activity, so the hot-reload probe (PROTOCOL.md step 9)
    // can tell "hooks unregistered" from "module torn down": flipping
    // plugins.entries.goalcup-capture.enabled=false mid-run should stop the llm_* posts;
    // whether it also stops the ticks is the actual answer to the row-8 unknown.
    if (Number.isFinite(TICK_MS) && TICK_MS > 0) {
      const timer = setInterval(() => {
        void post("plugin.tick", { uptimeSec: Math.round(process.uptime()) });
      }, TICK_MS);
      if (typeof timer.unref === "function") timer.unref();
      api.on("gateway_stop", async (event, ctx) => {
        clearInterval(timer);
        await post("gateway_stop", { ctx, event });
      });
    } else {
      api.on("gateway_stop", tap("gateway_stop"));
    }
    api.on("gateway_start", tap("gateway_start"));

    // ---- the four required taps ----------------------------------------------
    // llm_input carries the compiled system prompt; it is the ONLY place the system
    // prompt is observable (it is not written to transcript_events).
    api.on("llm_input", tap("llm_input"));
    api.on("llm_output", tap("llm_output"));
    api.on("before_tool_call", async (event, ctx) => {
      await post("before_tool_call", { ctx, event });
      // Never block a tool: this spike observes, it does not gate tools.
      return undefined;
    });
    api.on("after_tool_call", tap("after_tool_call"));
    api.on("message_received", tap("message_received"));

    // ---- the blocking hook (A7) ----------------------------------------------
    // before_agent_run is OpenClaw's input gate: its result type is
    // InputGateDecision | void = {outcome:"pass"} | {outcome:"block", reason, message?}.
    // `prompt` is hoisted to the top level of the post body so the shared
    // stub-receiver.py --block-over 4000 can answer with the Claude/Codex-style
    // {"decision":"block","reason":...} envelope, which is translated here.
    api.on("before_agent_run", async (event, ctx) => {
      const reply = await post("before_agent_run", {
        ctx,
        event,
        prompt: typeof event?.prompt === "string" ? event.prompt : "",
        promptLength: typeof event?.prompt === "string" ? event.prompt.length : null,
      });
      const decision = reply && typeof reply === "object" ? reply.decision : undefined;
      if (decision !== "block") return { outcome: "pass" };
      const reason = typeof reply.reason === "string" ? reply.reason : "blocked by tracker";
      void post("before_agent_run.decision", {
        ctx,
        enforced: GATE,
        reason,
        promptLength: typeof event?.prompt === "string" ? event.prompt.length : null,
      });
      if (!GATE) return { outcome: "pass" };
      return {
        outcome: "block",
        reason,
        message: "Blocked by the goal-cup tracker: message exceeds the length limit.",
        category: "goalcup_over_length",
      };
    });

    // ---- corroboration (cheap, and it answers A4/A5/A6 without a second leg) ---
    api.on("session_start", tap("session_start"));
    api.on("session_end", tap("session_end"));
    api.on("subagent_spawned", tap("subagent_spawned"));
    api.on("subagent_ended", tap("subagent_ended"));
    api.on("agent_end", tap("agent_end"));
    // before_compaction / after_compaction: a compaction during the run rewrites what the
    // model sees, so record it even though the script should not trigger one.
    api.on("before_compaction", tap("before_compaction"));
    api.on("after_compaction", tap("after_compaction"));
  },
};
