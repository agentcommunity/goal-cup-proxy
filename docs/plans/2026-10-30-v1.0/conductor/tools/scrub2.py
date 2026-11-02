# Conductor scrub: any run of 12+ characters of a key in env (CLAUDE_API_KEY, OPENAI_API_KEY) found in saved files is
# replaced. Prints file names and counts only, never a value.
import os, sys, glob
keys = [os.environ[n] for n in ("CLAUDE_API_KEY", "OPENAI_API_KEY", "DEEPSEEK_API_KEY", "GEMINI_API_KEY", "GH_TOKEN", "AI_GATEWAY_API_KEY", "DATABASE_URL", "GOALCUP_TEST_APPROVAL_KEY", "VERCEL_TOKEN", "CLOUDFLARE_API_TOKEN") if os.environ.get(n)]
N = 12
for f in glob.glob(sys.argv[1] + "/**/*", recursive=True):
    if not os.path.isfile(f) or f.endswith(".tar"):
        continue
    t = open(f, "rb").read().decode("utf-8", "surrogateescape"); orig = t; hits = 0
    for k in keys:
        grams = {k[i:i + N] for i in range(len(k) - N + 1)}
        i = 0
        while i <= len(t) - N:
            if t[i:i + N] in grams:
                j = i + N
                while j < len(t) and t[i:j + 1] in k:
                    j += 1
                t = t[:i] + "[KEY-FRAGMENT-REDACTED]" + t[j:]; hits += 1
            i += 1
    if t != orig:
        open(f, "wb").write(t.encode("utf-8", "surrogateescape")); print("scrubbed %d fragment(s) in %s" % (hits, f))
