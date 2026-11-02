#!/usr/bin/env bash
# Run every conductor check from the repo root. One PASS or FAIL line per file; exit 1 on any FAIL.
# Usage: bash test/run-all.sh
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)" || exit 2
T=$(cd "$(dirname "$0")" && pwd)
bad=0
for f in "$T"/test_*.sh "$T"/test_*.mjs "$T"/test_*.py; do
  [ -f "$f" ] || continue
  case $f in
    *.sh) cmd=(bash "$f") ;;
    *.mjs) cmd=(node "$f") ;;
    *.py) cmd=(python3 -m unittest discover -s "$T" -p "$(basename "$f")") ;;
  esac
  out=$("${cmd[@]}" 2>&1)
  if [ $? = 0 ]; then echo "PASS $(basename "$f")"; else echo "FAIL $(basename "$f")"; printf '%s\n' "$out" | tail -15 | sed 's/^/  /'; bad=1; fi
done
exit $bad
