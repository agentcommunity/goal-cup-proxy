# Counts only. Never prints a secret.
import json,glob,base64,re,os,hashlib
key=json.load(open("/vercel/.config/goalcup/capture-homes/codex-cli/auth.json"))["OPENAI_API_KEY"].encode()
tok=[l.split("=",1)[1].strip() for l in open("/vercel/proj/.env.local") if l.startswith("GH_TOKEN=")][0].encode()
print("key sha256 prefix", hashlib.sha256(key).hexdigest()[:12], "token sha256 prefix", hashlib.sha256(tok).hexdigest()[:12])
def scan(name, data):
    hits={"openai_key":data.count(key),"gh_token":data.count(tok),"key_last12":data.count(key[-12:]),"tok_last12":data.count(tok[-12:]),
          "authorization_hdr":len(re.findall(rb'(?i)authorization:\s*bearer\s+[A-Za-z0-9]',data)),"bearer_sk":len(re.findall(rb'Bearer sk-',data))}
    if any(hits.values()): print("HIT",name,hits)
    return hits
tot={}
T="/vercel/.config/goalcup-artifacts/artifacts/ede1ba278cd6261c8/trace"
files=glob.glob(T+"/*")+glob.glob("/vercel/.config/goalcup/runs/*/*")+glob.glob("/vercel/.config/goalcup/uploads/*")+glob.glob("/vercel/.config/goalcup/capture-homes/codex-cli/sessions/*/*/*/*.jsonl")+["/vercel/.config/goalcup/daemon.log"]
n=0
for fn in files:
    if fn.endswith("credential") or not os.path.isfile(fn): continue
    d=open(fn,"rb").read(); n+=1
    scan(fn,d)
    if "/trace/chain" in fn:
        for l in d.splitlines():
            for m in re.finditer(rb'"data_b64":"([A-Za-z0-9+/=]+)"',l):
                scan(fn+"#seg",base64.b64decode(m.group(1)))
print("files scanned",n)
