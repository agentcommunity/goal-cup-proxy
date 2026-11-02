# Report files under argv[1] holding any 12+ char fragment of a secret env value. Names and counts only.
import os, sys, glob
names = ("CLAUDE_API_KEY","OPENAI_API_KEY","GOALCUP_TEST_APPROVAL_KEY","GH_TOKEN","CLOUDFLARE_API_TOKEN","DATABASE_URL","VERCEL_TOKEN","GOALCUP_TEST_MEMBER_ID","AI_GATEWAY_API_KEY")
keys = [(n, os.environ[n]) for n in names if len(os.environ.get(n, "")) >= 12]
N = 12; bad = 0
for f in glob.glob(sys.argv[1] + "/**/*", recursive=True):
    if not os.path.isfile(f) or f.endswith(".py"): continue
    t = open(f, "rb").read().decode("utf-8", "surrogateescape")
    for n, k in keys:
        if any(k[i:i+N] in t for i in range(len(k)-N+1)):
            print("HIT", n, f); bad += 1
print("keyscan: %d hit(s), %d keys checked" % (bad, len(keys)))
