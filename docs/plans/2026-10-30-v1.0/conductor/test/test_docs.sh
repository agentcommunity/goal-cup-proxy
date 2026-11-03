. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); P="$C/STATE.md $C/SECRETS.md $C/snapshots.md $C/owner-decisions-draft.md $C/NEW-SESSION-PROMPT.md $C/reports/README.md $C/handoff-head.md"
# t_docs_exist
for f in $P; do [ -s "$f" ] || fail t_docs_exist "$f"; done; ok t_docs_exist
# t_no_em_dash
grep -n $'\xe2\x80\x94' $P && fail t_no_em_dash "" || ok t_no_em_dash
# t_no_model_ids (records and the owner-quote draft exempt)
grep -nEi 'claude-(haiku|opus|sonnet)|haiku|opus|sonnet|gpt-[0-9]|deepseek-v[0-9]|gemini-[0-9]' "$C/STATE.md" "$C/SECRETS.md" "$C/NEW-SESSION-PROMPT.md" "$C/reports/README.md" "$C/handoff-head.md" && fail t_no_model_ids "" || ok t_no_model_ids
# t_secrets_match_keyscan
want=$(python3 -c "import sys;sys.path.insert(0,'$C');import keyscan;print('\n'.join(sorted(keyscan.NAMES)))")
got=$(grep -oE '^- `[A-Z_]+`' "$C/SECRETS.md" | sed -E 's/^- `([A-Z_]+)`$/\1/' | sort)
[ "$want" = "$got" ] && ok t_secrets_match_keyscan || fail t_secrets_match_keyscan "$(diff <(echo "$want") <(echo "$got"))"
# t_draft_cites_ledger: every draft line's time is in the ledger copy (checked after Task 10 adds ledger/)
for t in $(grep -oE '[0-9]{2}:[0-9]{2}Z' "$C/owner-decisions-draft.md" | sort -u); do grep -q "^$t" "$C/ledger/integrator.log" 2>/dev/null || [ ! -d "$C/ledger" ] || fail t_draft_cites_ledger "$t"; done; ok t_draft_cites_ledger
# t_prompt_names_install_and_session
grep -q 'install.sh --dest' "$C/NEW-SESSION-PROMPT.md" && grep -q -- '--session' "$C/NEW-SESSION-PROMPT.md" && ! grep -q 'session_01V2' "$C/NEW-SESSION-PROMPT.md" && ok t_prompt_names_install_and_session || fail t_prompt_names_install_and_session ""
# t_patch_written_down
grep -q 'EnvHttpProxyAgent' "$C/handoff-head.md" && grep -q 'proxy-preload.cjs' "$C/handoff-head.md" && ok t_patch_written_down || fail t_patch_written_down ""
finish
