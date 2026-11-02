. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); d=$(tmp)
# fake pad worktree: a stub pad, an image id, connectors
mkdir -p "$d/wt/scripts/pad" "$d/wt/connectors" "$d/build" "$d/out"
printf 'echo "GH=${GH_TOKEN-unset} PNM=$PAD_NODE_MODULES NO=$NODE_OPTIONS"\n' > "$d/wt/scripts/pad/pad"
echo snap_test > "$d/wt/scripts/pad/image.id"; touch "$d/s1.toml" "$d/build/goalcup" "$d/build/goalcup-verify"
# stub sbx: records the sub-command, which secret names are in its environment, and the arguments; exec fails
cat > "$d/sbx" <<'SB'
echo "$1 $(env | cut -d= -f1 | sort | grep -E '^(GH_TOKEN|DATABASE_URL|CLAUDE_API_KEY|VERCEL_TOKEN)$' | tr '\n' ,) $*" >> "$(dirname "$0")/calls"
[ "$1" = exec ] && exit 1
exit 0
SB
export GC_SESSION=https://claude.ai/code/session_TESTabcd123
# t_padrun_env
GH_TOKEN=ghp_testtesttest GC_PAD_WT=$d/wt GC_PYTHON=bash bash "$C/padrun.sh" "$d/s1.toml" "$d/out" >/dev/null
grep -q 'GH=unset' "$d/out/logs/s1.log" && grep -q "PNM=$C/node_modules" "$d/out/logs/s1.log" && grep -q -- "--require $C/proxy-preload.cjs" "$d/out/logs/s1.log" && grep -q 'PAD_EXIT=0' "$d/out/logs/s1.log" && ok t_padrun_env || fail t_padrun_env ""
# t_padrun_needs_pad_wt
( unset GC_PAD_WT; bash "$C/padrun.sh" "$d/s1.toml" "$d/out" ) >/dev/null 2>&1; [ $? = 2 ] && ok t_padrun_needs_pad_wt || fail t_padrun_needs_pad_wt "not 2"
# t_padbox_cleans_up_on_failure: exec fails, rm still runs, and runs last
VERCEL_TOKEN=vt-test-not-real CLAUDE_API_KEY=sk-test-0123456789abcdef DATABASE_URL=postgresql://u:pw0123456789abc@h/db GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt GC_BUILD=$d/build bash "$C/padbox.sh" "$d/s1.toml" claude CLAUDE_API_KEY "$d/out" >/dev/null 2>&1
[ "$(tail -1 "$d/calls" | cut -d' ' -f1)" = rm ] && ok t_padbox_cleans_up_on_failure || fail t_padbox_cleans_up_on_failure ""
# t_padbox_no_value_in_argv
! grep -q 0123456789abcdef "$d/calls" && ! grep -q 'DATABASE_URL' "$d/calls" && ok t_padbox_no_value_in_argv || fail t_padbox_no_value_in_argv ""
# t_padbox_key_reaches_put_env: the key is in the environment of put-env only, by name
grep -q '^put-env CLAUDE_API_KEY,' "$d/calls" && ! grep -v '^put-env' "$d/calls" | grep -q 'CLAUDE_API_KEY,' && ok t_padbox_key_reaches_put_env || fail t_padbox_key_reaches_put_env ""
# t_padbox_vercel_token_reaches_sbx: sandbox credentials are kept for every call
! grep -v 'VERCEL_TOKEN,' "$d/calls" | grep -q . && grep -q '^create ' "$d/calls" && ok t_padbox_vercel_token_reaches_sbx || fail t_padbox_vercel_token_reaches_sbx ""
# t_padbox_provider_keys_only_in_put_env: model keys, GH_TOKEN, DATABASE_URL absent from every call except put-env's own key
! grep -v '^put-env' "$d/calls" | grep -qE 'CLAUDE_API_KEY,|GH_TOKEN,|DATABASE_URL,' && ok t_padbox_provider_keys_only_in_put_env || fail t_padbox_provider_keys_only_in_put_env ""
finish
