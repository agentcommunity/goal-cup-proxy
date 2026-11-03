. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); d=$(tmp)
# fake pad worktree holding the SQL heredoc of scripts/fetch-run.sh
mkdir -p "$d/wt/scripts" "$d/out"
cat > "$d/wt/scripts/fetch-run.sh" <<'FR'
#!/bin/sh
psql -X -q -At <<'SQL'
BEGIN READ ONLY;
SELECT 1 WHERE :'id' = :'id';
ROLLBACK;
SQL
FR
echo r0123456789abcdef > "$d/ids.txt"
# stub sbx: records the sub-command, which secret names are in its environment, and the arguments.
# get writes a tar holding idx/r1.json at the local path (the last argument).
cat > "$d/sbx" <<'SB'
echo "$1 $(env | cut -d= -f1 | grep -E '^(GH_TOKEN|DATABASE_URL|CLAUDE_API_KEY|VERCEL_TOKEN)$' | tr '\n' ,) $*" >> "$(dirname "$0")/calls"
if [ "$1" = get ]; then
  eval "last=\${$#}"; w=$(mktemp -d); mkdir -p "$w/idx"; echo '[]' > "$w/idx/r1.json"
  tar cf "$last" -C "$w" idx; rm -rf "$w"
fi
exit 0
SB
export GC_SESSION=https://claude.ai/code/session_TESTabcd123
# t_dbfetch_order
DATABASE_URL=postgresql://u:pw0123456789abc@h/db GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt bash "$C/dbfetch/run.sh" "$d/ids.txt" "$d/out" >/dev/null 2>&1
[ "$(cut -d' ' -f1 "$d/calls" | tr '\n' ' ')" = "create put put put put exec put-env exec get rm " ] && [ -f "$d/out/idx/r1.json" ] && ok t_dbfetch_order || fail t_dbfetch_order "$(cut -d' ' -f1 "$d/calls" | tr '\n' ' ')"
# t_dburl_only_in_put_env
[ "$(grep -c 'DATABASE_URL,' "$d/calls")" = 1 ] && grep -q '^put-env DATABASE_URL,' "$d/calls" && ! grep -q pw0123456789abc "$d/calls" && ok t_dburl_only_in_put_env || fail t_dburl_only_in_put_env ""
# t_refuses_write_sql
sed -i 's/BEGIN READ ONLY;/BEGIN;/' "$d/wt/scripts/fetch-run.sh"; : > "$d/calls"
GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt bash "$C/dbfetch/run.sh" "$d/ids.txt" "$d/out2" >/dev/null 2>&1; [ $? = 4 ] && [ ! -s "$d/calls" ] && ok t_refuses_write_sql || fail t_refuses_write_sql ""
finish
