. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); d=$(tmp)
fake_sdk() { mkdir -p "$1/@vercel/sandbox/dist/api-client" "$1/undici"; printf '%s\n' 'import { APIError } from "./api-error.js";' "$2" > "$1/@vercel/sandbox/dist/api-client/base-client.js"
  echo 'class A{}; class E{}; module.exports={Agent:A, EnvHttpProxyAgent:E};' > "$1/undici/index.js"; }
# t_patch_rewrites_import
fake_sdk "$d/nm" 'import { Agent } from "undici";'; bash "$C/patch-sandbox.sh" "$d/nm" | grep -qx patched && grep -qxF 'import { EnvHttpProxyAgent as Agent } from "undici";' "$d/nm/@vercel/sandbox/dist/api-client/base-client.js" && ok t_patch_rewrites_import || fail t_patch_rewrites_import ""
# t_patch_idempotent
bash "$C/patch-sandbox.sh" "$d/nm" | grep -qx 'already patched' && ok t_patch_idempotent || fail t_patch_idempotent ""
# t_patch_unknown_layout_exit5
fake_sdk "$d/nm2" 'import { Agent as X } from "undici-next";'; bash "$C/patch-sandbox.sh" "$d/nm2" 2>"$d/err"; [ $? = 5 ] && grep -q 'unknown @vercel/sandbox layout' "$d/err" && ok t_patch_unknown_layout_exit5 || fail t_patch_unknown_layout_exit5 ""
# t_preload_swaps_agent
mkdir -p "$d/pre"; cp "$C/proxy-preload.cjs" "$d/pre/"; fake_sdk "$d/pre/node_modules" x
node --require "$d/pre/proxy-preload.cjs" -e "const u=require('$d/pre/node_modules/undici'); process.exit(u.Agent===u.EnvHttpProxyAgent?0:1)" && ok t_preload_swaps_agent || fail t_preload_swaps_agent ""
# t_install_env_has_no_secret: env.sh holds the given paths, never a secret value
fake_sdk "$C/node_modules" 'import { Agent } from "undici";' 2>/dev/null || true
CLAUDE_API_KEY=sk-test-0123456789abcdef bash "$C/install.sh" --dest "$d/tools" --repo "$d" --night "$d/night" --session https://claude.ai/code/session_TESTabcd123 --skip-npm
grep -q "GC_SESSION=https://claude.ai/code/session_TESTabcd123" "$d/tools/env.sh" && ! grep -q 0123456789abcdef "$d/tools/env.sh" && [ ! -e "$d/tools/test" ] && ok t_install_env_has_no_secret || fail t_install_env_has_no_secret ""
# t_env_sh_defaults_only: the caller's GC_SESSION beats the one written at install time
r=$(GC_SESSION=https://claude.ai/code/session_OTHER bash -c ". '$d/tools/env.sh'; echo \$GC_SESSION"); u=$(env -u GC_SESSION bash -c ". '$d/tools/env.sh'; echo \$GC_SESSION")
[ "$r" = "https://claude.ai/code/session_OTHER" ] && [ "$u" = "https://claude.ai/code/session_TESTabcd123" ] && ok t_env_sh_defaults_only || fail t_env_sh_defaults_only "$r $u"
# t_night_restore_per_pair: a kept local log never gets a committed .scribed
mkdir -p "$d/bare2" && cp -r "$C"/. "$d/bare2/" && rm -rf "$d/bare2/ledger" && mkdir "$d/bare2/ledger" && echo a > "$d/bare2/ledger/x.log" && echo 5 > "$d/bare2/ledger/x.log.scribed"
mkdir -p "$d/n2/lanes" && echo local > "$d/n2/lanes/x.log"
bash "$d/bare2/install.sh" --dest "$d/tools4" --night "$d/n2" --skip-npm >/dev/null
[ "$(cat "$d/n2/lanes/x.log")" = local ] && [ ! -e "$d/n2/lanes/x.log.scribed" ] && ok t_night_restore_per_pair || fail t_night_restore_per_pair ""
# t_installed_help_runs
node "$d/tools/box.mjs" --help >/dev/null && ok t_installed_help_runs || fail t_installed_help_runs ""
# t_skip_npm_without_sdk_ok: a clean worktree has no node_modules
cp -r "$C" "$d/bare"; rm -rf "$d/bare/node_modules"
bash "$d/bare/install.sh" --dest "$d/tools3" --skip-npm | grep -qx 'patch skipped: no node_modules' && ok t_skip_npm_without_sdk_ok || fail t_skip_npm_without_sdk_ok ""
finish
