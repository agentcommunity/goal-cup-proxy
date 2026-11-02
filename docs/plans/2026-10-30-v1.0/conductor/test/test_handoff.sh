. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); d=$(tmp)
CR=docs/plans/2026-10-30-v1.0/conductor
S=https://claude.ai/code/session_TESTabcd123
fake_repo "$d"
bash "$C/install.sh" --dest "$d/tools" --skip-npm >/dev/null
N=$d/night
mkdir -p "$N/lanes" "$N/gw3/pad-out" "$N/finish" "$N/reviews" "$N/backlog"
printf 'a\nb\nc\n' > "$N/lanes/integrator.log"; echo 2 > "$N/lanes/integrator.log.scribed"
echo line > "$N/lanes/pad-model.log"; echo p > "$N/lanes/x.log.pending"
echo report > "$N/gw3/REPORT.txt"; echo hidden > "$N/gw3/pad-out/secret.txt"; echo hidden > "$N/gw3/.env"
echo plan > "$N/finish/FINISH-PLAN.md"; echo rev > "$N/reviews/LATEST.md"; echo pad > "$N/backlog/next-pad-verify.md"
# t_handoff_bundle
GC_SESSION=$S GC_REPO=$d/repo GC_NIGHT=$N GC_KEYSCAN_ALLOW_EMPTY=1 bash "$d/tools/handoff.sh" >/dev/null
git -C "$d/repo" fetch -q origin
T=$(git -C "$d/repo" ls-tree -r --name-only origin/v1.0/integration -- "$CR")
miss=
for f in ledger/integrator.log ledger/integrator.log.scribed ledger/pad-model.log reports/gw3.txt FINISH-PLAN.md mq-driver.sh keyscan.py package-lock.json; do echo "$T" | grep -qx "$CR/$f" || miss="$miss $f"; done
if [ -n "$miss" ]; then fail t_handoff_bundle "missing$miss"
elif echo "$T" | grep -qE 'pad-out|\.env|\.pending|node_modules'; then fail t_handoff_bundle "copied a forbidden file"
else ok t_handoff_bundle; fi
# t_handoff_trailer
git -C "$d/repo" log -1 --format='%(trailers:key=Claude-Session,valueonly)' origin/v1.0/integration | grep -qx "$S" && ok t_handoff_trailer || fail t_handoff_trailer ""
# t_handoff_keyscan_refuses: a planted value stops it, nothing pushed
before=$(git -C "$d/repo" rev-parse origin/v1.0/integration); echo "k=sk-test-0123456789abcdef" >> "$N/lanes/integrator.log"
CLAUDE_API_KEY=sk-test-0123456789abcdef GC_SESSION=$S GC_REPO=$d/repo GC_NIGHT=$N bash "$d/tools/handoff.sh" >/dev/null 2>&1; rc=$?
git -C "$d/repo" fetch -q origin
[ $rc = 6 ] && [ "$(git -C "$d/repo" rev-parse origin/v1.0/integration)" = "$before" ] && ok t_handoff_keyscan_refuses || fail t_handoff_keyscan_refuses "rc=$rc"
# t_restore_keeps_scribed: restore from the pushed tree into an empty night
git -C "$d/repo" worktree add -q --detach "$d/new" origin/v1.0/integration
bash "$d/new/$CR/install.sh" --dest "$d/tools2" --night "$d/night2" --skip-npm >/dev/null
[ "$(cat "$d/night2/lanes/integrator.log.scribed")" = 2 ] && [ ! -e "$d/night2/lanes/pad-model.log.scribed" ] && [ -d "$d/night2/mrq/done" ] && [ -e "$d/night2/integration.lock" ] && ok t_restore_keeps_scribed || fail t_restore_keeps_scribed ""
# t_restore_never_overwrites
echo local > "$d/night2/lanes/integrator.log"
bash "$d/new/$CR/install.sh" --dest "$d/tools2" --night "$d/night2" --skip-npm | grep -q 'kept existing integrator.log' && [ "$(cat "$d/night2/lanes/integrator.log")" = local ] && ok t_restore_never_overwrites || fail t_restore_never_overwrites ""
finish
