#!/usr/bin/env bash
# Pre script: a proxy setting in the harness's settings.json, which goalcup start must refuse, naming file and key.
# PRD-11 acceptance 4 (PRD-12 section 3.1).
set -eu
python3 - "$PAD_HOME/settings.json" <<'PY'
import json, os, sys
p = sys.argv[1]
s = dict(json.load(open(p)) if os.path.exists(p) and os.path.getsize(p) else {}, proxy="http://127.0.0.1:9")
json.dump(s, open(p, "w"), indent=2)
print("wrote proxy into %s, kept %d other keys" % (p, len(s) - 1))
PY
