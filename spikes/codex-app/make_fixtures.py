#!/usr/bin/env python3
"""Build the committed fixtures for spike 3 (codex-app) from the artifact root.

  python3 make_fixtures.py --root <artifact-root>

Structural scrub first (things a regex cannot see: the user's memory summary, the
skill listing, the plugin listing, billing/rate-limit state), then spikes/redact.py
over every output, then an eye review. Every scrub is logged to
<root>/analysis/fixture-scrub-log.json and summarised in fixtures/README.md.
"""
import argparse, hashlib, inspect, json, os, shutil, subprocess, sys, tempfile

SPIKE = os.path.dirname(os.path.abspath(__file__))
MAIN_ID = "01a06657-c53d-7201-afa1-cefdaac2edf5"
PROBE_ID = "01a0666d-87ec-7d12-b578-6d79f921376b"
SUB_ID = "01a06658-688f-7790-8124-1635bbceddc8"
A9_FILE = "rollout-2026-09-03T15-24-06-01a0665e-3a4f-7850-9c91-6feab6752664.jsonl"
A9_OUTPUT = "rollout-a9-foreign-01a0665e-excerpt.jsonl"
A9_ORDINALS = {0, 1, 7, 8, 9, 10, 11, 12, 13}
A9_EXPECTED_SIZE = 5752
A9_EXPECTED_SHA256 = "e69d1dee76bcfcf6df9b097a6a447b0d75bf50214763a3f0a39ac39bb77d5206"

LOG = []


def note(f, field, ordinal, removed, why):
    LOG.append({"file": f, "field": field, "ordinal": ordinal, "removed_chars": removed, "why": why})


def ph(n, what):
    return f"<goalcup-scrubbed len={n} {what}>"


def already_scrubbed(value):
    return isinstance(value, str) and "<goalcup-scrubbed len=" in value


def a9_identity_from_source(src):
    """Return the foreign cwd and repository URL declared by the source rollout."""
    for line in open(src, encoding="utf-8"):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get("type") != "session_meta":
            continue
        pl = r.get("payload") or {}
        git = pl.get("git") or {}
        cwd, repo = pl.get("cwd"), git.get("repository_url")
        if isinstance(cwd, str) and isinstance(repo, str):
            if already_scrubbed(cwd) and already_scrubbed(repo):
                return None
            return cwd, repo
        break
    raise ValueError(f"A9 source has no complete session identity: {src}")


def scrub_a9_identity(r, fname, identity):
    """Replace exact A9 cwd/repository identities everywhere in one record."""
    private_cwd, private_repo = identity
    ordinal = r.get("ordinal")

    def walk(value, field):
        if isinstance(value, str):
            if value == private_cwd:
                note(fname, field, ordinal, len(value), "private project path")
                return ph(len(value), "private project path")
            if value == private_repo:
                note(fname, field, ordinal, len(value), "private repository URL")
                return ph(len(value), "private repository URL")
            return value
        if isinstance(value, list):
            return [walk(item, f"{field}[]") for item in value]
        if isinstance(value, dict):
            return {
                key: walk(item, f"{field}.{key}" if field else key)
                for key, item in value.items()
            }
        return value

    return walk(r, "")


def msg_text(pl):
    c = pl.get("content")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "".join(x.get("text", "") for x in c if isinstance(x, dict))
    return ""


def set_msg_text(pl, new):
    c = pl.get("content")
    if isinstance(c, list):
        for x in c:
            if isinstance(x, dict) and "text" in x:
                x["text"] = new
                # collapse any further text parts
        pl["content"] = [x for i, x in enumerate(c) if not (isinstance(x, dict) and "text" in x and i > 0)]
    else:
        pl["content"] = new


def scrub_record(r, fname):
    """Structural scrub of one rollout record, in place. Returns the record."""
    t = r.get("type")
    pl = r.get("payload")
    if not isinstance(pl, dict):
        return r
    o = r.get("ordinal")

    if t == "session_meta":
        bi = pl.get("base_instructions")
        if isinstance(bi, dict) and isinstance(bi.get("text"), str) and not already_scrubbed(bi["text"]):
            n = len(bi["text"])
            bi["text"] = ph(n, "stock Codex base instructions (identical in every rollout)")
            note(fname, "session_meta.base_instructions.text", o, n, "stock system prompt, bulk only")

    if t == "world_state":
        st = (pl.get("state") or {})
        hs = st.get("host_skills")
        if isinstance(hs, dict) and isinstance(hs.get("body"), str) and not already_scrubbed(hs["body"]):
            n = len(hs["body"])
            hs["body"] = ph(n, "the operator's skill listing: names, descriptions and absolute paths of private project skills")
            note(fname, "world_state.state.host_skills.body", o, n, "private skill inventory")
        pers = st.get("personality")
        if (isinstance(pers, dict) and isinstance(pers.get("instructions"), str)
                and len(pers["instructions"]) > 400 and not already_scrubbed(pers["instructions"])):
            n = len(pers["instructions"])
            pers["instructions"] = ph(n, "stock personality prompt")
            note(fname, "world_state.state.personality.instructions", o, n, "stock prompt, bulk only")

    if t == "event_msg" and pl.get("type") == "token_count":
        rl = pl.get("rate_limits")
        if isinstance(rl, dict) and "_scrubbed" not in rl:
            keep = {"limit_id": rl.get("limit_id")}
            n = len(json.dumps(rl))
            pl["rate_limits"] = {**keep, "_scrubbed": ph(n, "plan_type, credit balance, quota used_percent, window and reset time")}
            note(fname, "event_msg/token_count.rate_limits", o, n, "account plan, credit balance and quota state")

    if t == "event_msg" and pl.get("type") == "item_completed":
        item = pl.get("item")
        if isinstance(item, dict):
            for k in ("stdout", "stderr", "aggregated_output", "formatted_output", "output", "text"):
                v = item.get(k)
                if isinstance(v, str) and len(v) > 3000 and not already_scrubbed(v):
                    n = len(v)
                    item[k] = ph(n, "the UI mirror of the same tool output (the operator's local SKILL.md files)")
                    note(fname, f"event_msg/item_completed.item.{k}", o, n,
                         "duplicate of the scrubbed tool output, in the app's UI event stream")

    if t == "response_item" and pl.get("type") == "message":
        txt = msg_text(pl)
        role = pl.get("role")
        if "<goalcup-scrubbed len=" not in txt and txt.startswith("<app-context>"):
            n = len(txt)
            set_msg_text(pl, ph(n, "the app's desktop context block: it embeds ~/.codex/memories/memory_summary.md "
                                   "(the operator's private projects, clients and preferences), the skill listing "
                                   "with absolute paths, and the collaboration/apps/plugins instruction blocks"))
            note(fname, f"response_item/message role={role}", o, n, "app-context: private memory summary + skill listing")
        elif "<goalcup-scrubbed len=" not in txt and txt.startswith("<recommended_plugins>"):
            n = len(txt)
            set_msg_text(pl, txt[:200] + ph(n - 200, "the rest of the recommended-plugin list (the operator's "
                                                     "available connectors) and the <environment_context> block that "
                                                     "is concatenated into the SAME role=user message"))
            note(fname, f"response_item/message role={role}", o, n - 200,
                 "recommended_plugins + environment_context; first 200 chars kept (the shape is the evidence)")
        elif "<goalcup-scrubbed len=" not in txt and txt.startswith("<skills_instructions>"):
            n = len(txt)
            set_msg_text(pl, ph(n, "the operator's skill listing with absolute paths"))
            note(fname, f"response_item/message role={role}", o, n, "private skill inventory")
        elif "<goalcup-scrubbed len=" not in txt and txt.startswith("<model_switch>"):
            n = len(txt)
            set_msg_text(pl, txt[:200] + ph(n - 200, "the rest of the re-issued stock system prompt for the new model"))
            note(fname, f"response_item/message role={role}", o, n - 200, "stock prompt, bulk only; marker kept")

    if t == "response_item" and pl.get("type") in ("custom_tool_call_output", "function_call_output"):
        out = pl.get("output")
        s = out if isinstance(out, str) else json.dumps(out)
        if len(s) > 3000 and not already_scrubbed(s):
            n = len(s)
            pl["output"] = ph(n, "tool output: the contents of the operator's local SKILL.md files")
            note(fname, "response_item/*_output.output", o, n, "bulk tool output quoting the operator's skill files")

    if t == "response_item" and pl.get("type") == "function_call" and pl.get("name") == "spawn_agent":
        a = pl.get("arguments")
        if isinstance(a, str) and len(a) > 300 and not already_scrubbed(a):
            d = json.loads(a)
            if isinstance(d.get("message"), str) and len(d["message"]) > 80:
                n = len(d["message"])
                d["message"] = d["message"][:60] + ph(n - 60, "Fernet-encrypted subagent task payload (unreadable either way)")
                pl["arguments"] = json.dumps(d)
                note(fname, "response_item/function_call spawn_agent.arguments.message", o, n - 60,
                     "encrypted blob, truncated for size; it is opaque in the original too")

    if t == "response_item" and pl.get("type") == "agent_message":
        c = pl.get("content")
        if isinstance(c, list):
            for x in c:
                if (isinstance(x, dict) and isinstance(x.get("encrypted_content"), str)
                        and len(x["encrypted_content"]) > 80
                        and not already_scrubbed(x["encrypted_content"])):
                    n = len(x["encrypted_content"])
                    x["encrypted_content"] = x["encrypted_content"][:60] + ph(n - 60, "Fernet-encrypted task payload")
                    note(fname, "response_item/agent_message.content[].encrypted_content", o, n - 60,
                         "encrypted blob, truncated for size")
    return r



def scrub_hook(h, fname):
    """Structural scrub of one leg-2 hook post, in place."""
    b = h.get("body")
    if not isinstance(b, dict):
        return h
    ev = b.get("hook_event_name")
    for k in ("tool_response", "last_assistant_message"):
        v = b.get(k)
        s = v if isinstance(v, str) else None
        if s and len(s) > 3000:
            n = len(s)
            b[k] = ph(n, "tool output quoting the operator's local SKILL.md files")
            note(fname, f"{ev}.{k}", None, n, "bulk tool output quoting the operator's skill files")
    return h


def build_hooks(src, dst, fname):
    out = []
    for line in open(src, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        out.append(json.dumps(scrub_hook(json.loads(line), fname), ensure_ascii=False))
    tmp = dst + ".pre-redact"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(out) + "\n")
    subprocess.run([sys.executable, os.path.join(SPIKE, "..", "redact.py"), tmp, dst], check=True)
    os.remove(tmp)
    return dst


def build(src, dst, fname, keep_ordinals=None, identity=None):
    out = []
    for line in open(src, encoding="utf-8"):
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        if keep_ordinals is not None and r.get("ordinal") not in keep_ordinals:
            continue
        r = scrub_record(r, fname)
        if identity is not None:
            r = scrub_a9_identity(r, fname, identity)
        out.append(json.dumps(r, ensure_ascii=False))
    tmp = dst + ".pre-redact"
    with open(tmp, "w", encoding="utf-8") as fh:
        fh.write("\n".join(out) + "\n")
    subprocess.run([sys.executable, os.path.join(SPIKE, "..", "redact.py"), tmp, dst], check=True)
    os.remove(tmp)
    return dst


def build_a9(src, dst):
    """Build the A9 fixture through the same identity-wiring path as main."""
    return build(src, dst, A9_OUTPUT, keep_ordinals=A9_ORDINALS,
                 identity=a9_identity_from_source(src))


def remove_legacy_a9_sidecars(fx):
    """Drop stale A9 sidecars whose names predate the public fixture identity."""
    current = A9_OUTPUT + ".redactions.jsonl"
    for name in os.listdir(fx):
        if name.startswith("rollout-a9-") and name.endswith(".jsonl.redactions.jsonl") and name != current:
            os.remove(os.path.join(fx, name))


def self_check():
    """Verify the checked-in A9 bytes and the documented regeneration path."""
    fx = os.path.join(SPIKE, "fixtures")
    assert "build_a9(" in inspect.getsource(main), "main must use the production A9 builder"
    fixture = os.path.join(fx, A9_OUTPUT)
    sidecar = fixture + ".redactions.jsonl"
    assert os.path.isfile(fixture), f"missing fixture: {fixture}"
    assert os.path.isfile(sidecar), f"missing sidecar: {sidecar}"
    stale = [
        name for name in os.listdir(fx)
        if name.startswith("rollout-a9-")
        and name.endswith(".jsonl.redactions.jsonl")
        and name != os.path.basename(sidecar)
    ]
    assert not stale, f"stale A9 sidecars remain: {stale}"
    raw = open(fixture, "rb").read()
    assert len(raw) == A9_EXPECTED_SIZE, len(raw)
    assert hashlib.sha256(raw).hexdigest() == A9_EXPECTED_SHA256

    records = [json.loads(line) for line in raw.decode().splitlines()]
    assert [r["ordinal"] for r in records] == sorted(A9_ORDINALS)
    assert len(records) == len(A9_ORDINALS)
    marker_cwd = ph(34, "private project path")
    marker_repo = ph(35, "private repository URL")
    by_ordinal = {record["ordinal"]: record for record in records}
    meta = by_ordinal[0]["payload"]
    turn = by_ordinal[7]["payload"]
    assert meta["cwd"] == marker_cwd
    assert meta["git"]["repository_url"] == marker_repo
    assert turn["cwd"] == marker_cwd
    assert marker_cwd in turn["workspace_roots"]
    assert any(root.startswith("/Users/team/.codex/") for root in turn["workspace_roots"])

    synthetic_cwd = "/Users/example/dev/PROJECTS/foreign-project"
    synthetic_repo = "https://github.com/example/foreign-project"
    transformed = scrub_a9_identity(
        {"ordinal": 0, "payload": {"cwd": synthetic_cwd, "git": {"repository_url": synthetic_repo}}},
        A9_OUTPUT,
        (synthetic_cwd, synthetic_repo),
    )
    assert transformed["payload"]["cwd"] == ph(len(synthetic_cwd), "private project path")
    assert transformed["payload"]["git"]["repository_url"] == ph(len(synthetic_repo), "private repository URL")

    synthetic_records = [
        {"ordinal": 0, "type": "session_meta", "payload": {
            "cwd": synthetic_cwd, "git": {"repository_url": synthetic_repo}
        }},
        {"ordinal": 7, "type": "turn_context", "payload": {
            "cwd": synthetic_cwd, "workspace_roots": [synthetic_cwd, "/Users/example/.codex"]
        }},
    ]
    with tempfile.TemporaryDirectory() as tmp:
        synthetic = os.path.join(tmp, "synthetic-a9.jsonl")
        with open(synthetic, "w", encoding="utf-8") as fh:
            for record in synthetic_records:
                fh.write(json.dumps(record) + "\n")
        synthetic_out = os.path.join(tmp, A9_OUTPUT)
        build_a9(synthetic, synthetic_out)
        generated_records = [json.loads(line) for line in open(synthetic_out, encoding="utf-8")]
        generated_by_ordinal = {record["ordinal"]: record for record in generated_records}
        generated_meta = generated_by_ordinal[0]["payload"]
        generated_turn = generated_by_ordinal[7]["payload"]
        assert generated_meta["cwd"] == ph(len(synthetic_cwd), "private project path")
        assert generated_meta["git"]["repository_url"] == ph(len(synthetic_repo), "private repository URL")
        assert generated_turn["cwd"] == ph(len(synthetic_cwd), "private project path")
        assert generated_turn["workspace_roots"] == [
            ph(len(synthetic_cwd), "private project path"), "/Users/example/.codex"
        ]
        generated_text = open(synthetic_out, encoding="utf-8").read()
        assert synthetic_cwd not in generated_text
        assert synthetic_repo not in generated_text

        regenerated = os.path.join(tmp, A9_OUTPUT)
        build_a9(fixture, regenerated)
        assert open(regenerated, "rb").read() == raw
        assert open(regenerated + ".redactions.jsonl", "rb").read() == open(sidecar, "rb").read()
    print(f"A9 self-check: {len(raw)} bytes sha256={A9_EXPECTED_SHA256}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root")
    ap.add_argument("--self-check", action="store_true")
    a = ap.parse_args()
    if a.self_check:
        self_check()
        return
    if not a.root:
        ap.error("--root is required unless --self-check is used")
    R = a.root
    FX = os.path.join(SPIKE, "fixtures")
    os.makedirs(FX, exist_ok=True)
    remove_legacy_a9_sidecars(FX)
    os.makedirs(os.path.join(R, "analysis"), exist_ok=True)
    sess = os.path.join(R, "store-copy", "sessions", "2026", "09", "03")
    home = os.path.expanduser("~")

    main_src = [f for f in os.listdir(sess) if MAIN_ID in f][0]
    sub_src = [f for f in os.listdir(sess) if SUB_ID in f][0]
    build(os.path.join(sess, main_src), os.path.join(FX, "rollout-main-01a06657.jsonl"),
          "rollout-main-01a06657.jsonl")
    build(os.path.join(sess, sub_src), os.path.join(FX, "rollout-subagent-01a06658.jsonl"),
          "rollout-subagent-01a06658.jsonl")
    # A9 probe: session_meta + the "say hello" turn only (ordinals 0,1,7,8,9,10,11,12,13);
    # ordinals 2-6 are the injected developer/user context and are NOT copied out at all.
    a9_src = os.path.join(home, ".codex", "sessions", "2026", "09", "03", A9_FILE)
    build_a9(a9_src, os.path.join(FX, A9_OUTPUT))
    note(A9_OUTPUT, "ordinals 2-6", None, 62434,
         "excerpt only: the <app-context>/<recommended_plugins> injection for an unrelated project "
         "is not copied out of ~/.codex at all")

    # ---- probe P3 ----
    probe_src = [f for f in os.listdir(sess) if PROBE_ID in f][0]
    build(os.path.join(sess, probe_src), os.path.join(FX, "rollout-probe-01a0666d.jsonl"),
          "rollout-probe-01a0666d.jsonl")
    build_hooks(os.path.join(R, "probe-hooks.jsonl"), os.path.join(FX, "probe-hooks.jsonl"),
                "probe-hooks.jsonl")
    wit2 = [f for f in os.listdir(os.path.join(R, "exception")) if f.startswith("config.toml.witness-before-cli-trust")][0]
    d2 = subprocess.run(["diff", "-u", os.path.join(R, "exception", wit2), os.path.join(home, ".codex", "config.toml")],
                        capture_output=True, text=True).stdout.replace(home, "$HOME")
    hdr2 = ("# ~/.codex/config.toml: witness taken 2026-09-03T08:36:41Z (before the human ran the\n"
            "# standalone Codex CLI 0.153.0 in the workspace and trusted the 12 hooks with /hooks)\n"
            "# vs the same file after. Paths rewritten to $HOME. sha256 cfb6a96b… -> 534befa1….\n"
            "# The write is append-only, made by Codex's own trust UI, and contains no secret:\n"
            "# every added value is a sha256 of a hook definition. The full file is NOT committed.\n")
    open(os.path.join(FX, "config-toml-cli-trust.diff"), "w").write(hdr2 + d2)

    ws = open(os.path.join(R, "workspace-path.txt")).read().strip()
    shutil.copyfile(os.path.join(ws, ".codex", "hooks.json"), os.path.join(FX, "tracker-hooks.json"))
    shutil.copyfile(os.path.join(ws, ".codex", "hooks", "hook-post.sh"), os.path.join(FX, "tracker-hook-post.sh"))
    shutil.copyfile(os.path.join(R, "hooks.jsonl"), os.path.join(FX, "hooks.jsonl"))

    # config.toml witness diff, NOT the file: $HOME-relative, and only the hunks.
    exc = os.path.join(R, "exception")
    wit = [f for f in os.listdir(exc) if f.startswith("config.toml.witness-before-ui-trust")][0]
    # end-of-main-run state == the witness taken just before the P3 CLI trust, so the main run's
    # A10 diff stays frozen and does not absorb the probe's twelve hooks.state tables.
    wit_end = [f for f in os.listdir(exc) if f.startswith("config.toml.witness-before-cli-trust")][0]
    d = subprocess.run(["diff", "-u", os.path.join(exc, wit), os.path.join(exc, wit_end)],
                       capture_output=True, text=True).stdout
    d = d.replace(home, "$HOME")
    hdr = ("# ~/.codex/config.toml across the MAIN RUN only: witness 2026-09-03T08:14:07Z (before the\n"
           "# human looked for a hooks UI) vs witness 2026-09-03T08:36:41Z (end of the main run, taken\n"
           "# just before probe P3). sha256 c41b4421... -> cfb6a96b.... Paths rewritten to $HOME.\n"
           "# Probe P3's separate change is in config-toml-cli-trust.diff. The full file is NOT\n"
           "# committed: it holds the operator's MCP server commands, URLs and env blocks.\n")
    open(os.path.join(FX, "config-toml-trust.diff"), "w").write(hdr + d)

    json.dump(LOG, open(os.path.join(R, "analysis", "fixture-scrub-log.json"), "w"), indent=1)
    print(f"{len(LOG)} structural scrubs -> {R}/analysis/fixture-scrub-log.json")
    for f in sorted(os.listdir(FX)):
        p = os.path.join(FX, f)
        if os.path.isfile(p):
            h = hashlib.sha256(open(p, "rb").read()).hexdigest()
            print(f"  {os.path.getsize(p):>9}  {h[:8]}…  {f}")


if __name__ == "__main__":
    main()
