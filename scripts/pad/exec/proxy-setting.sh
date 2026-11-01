#!/usr/bin/env bash
# Pre script: a proxy setting in the harness's settings.json, which goalcup start must refuse, naming file and key.
# PRD-11 acceptance 4 (PRD-12 section 3.1).
set -eu
python3 - "$PAD_HOME/settings.json" "$PAD_PROXY" <<'PY'
import json, os, sys
p = sys.argv[1]
if os.path.exists(p) and os.path.getsize(p):
    with open(p) as f:
        s = json.load(f)
else:
    s = {}
n = len(s) - ("proxy" in s)
s["proxy"] = "http://" + sys.argv[2]  # pad-run names the value, so it can look for it on the screen
with open(p, "w") as f:
    json.dump(s, f, indent=2)
print("wrote proxy into %s, kept %d other keys" % (p, n))
PY
