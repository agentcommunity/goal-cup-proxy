. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd)
# t_no_root_paths
if find "$C" -type f \( -name '*.sh' -o -name '*.mjs' -o -name '*.js' -o -name '*.cjs' -o -name '*.py' \) -not -path '*/node_modules/*' -not -path '*/test/*' -exec grep -nHE '/root/|/home/user|session_01V2' {} + ; then fail t_no_root_paths "literal path or old session"; else ok t_no_root_paths; fi
# t_trailer_needs_session
( unset GC_SESSION; . "$C/lib.sh"; gc_trailer ) 2>/dev/null && fail t_trailer_needs_session "returned 0" || ok t_trailer_needs_session
( GC_SESSION=https://example.com/x; . "$C/lib.sh"; gc_trailer ) 2>/dev/null && fail t_trailer_rejects_other_url "returned 0" || ok t_trailer_rejects_other_url
# t_trailer_text
out=$(GC_SESSION=https://claude.ai/code/session_TESTabcd123 bash -c ". '$C/lib.sh'; gc_trailer")
[ "$out" = $'\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_TESTabcd123' ] && ok t_trailer_text || fail t_trailer_text "$out"
# t_merge_refuses_without_session: exit 2, no worktree, no ledger line
d=$(tmp); fake_repo "$d"; mkdir -p "$d/night/lanes"
( unset GC_SESSION; GC_REPO=$d/repo GC_NIGHT=$d/night bash "$C/merge.sh" v1.0/x "m" ) >/dev/null 2>&1; rc=$?
[ $rc = 2 ] && [ ! -e "$d/night/merge-wt" ] && [ ! -s "$d/night/lanes/integrator.log" ] && ok t_merge_refuses_without_session || fail t_merge_refuses_without_session "rc=$rc"
# t_scribe_refuses_without_session
( unset GC_SESSION; GC_REPO=$d/repo GC_NIGHT=$d/night bash "$C/scribe.sh" ) >/dev/null 2>&1; [ $? = 2 ] && ok t_scribe_refuses_without_session || fail t_scribe_refuses_without_session "not 2"
# t_mergeq_runs_first_request: a copied tool folder with a stub merge.sh; result lands in GC_NIGHT/mrq
cp -r "$C" "$d/tools"; printf '#!/bin/bash\necho "stub merged $1"; exit 0\n' > "$d/tools/merge.sh"; chmod +x "$d/tools/merge.sh"
mkdir -p "$d/night/mrq/done"; echo '{"branch":"v1.0/x","message":"m","first":true}' > "$d/night/mrq/a1.req"
echo 'not json' > "$d/night/mrq/bad.req"
GC_REPO=$d/repo GC_NIGHT=$d/night timeout 60 bash "$d/tools/mergeq.sh" 1
for i in $(seq 30); do [ -f "$d/night/mrq/a1.res" ] && break; sleep 1; done
python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));assert d["exit"]==0 and "stub merged v1.0/x" in d["output"]' "$d/night/mrq/a1.res" && [ -f "$d/night/mrq/done/a1.req" ] && ok t_mergeq_runs_first_request || fail t_mergeq_runs_first_request "no result"
[ -f "$d/night/mrq/done/bad.req" ] && ok t_mergeq_bad_request_to_done || fail t_mergeq_bad_request_to_done "bad request left in queue"
finish
