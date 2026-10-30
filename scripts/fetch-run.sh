#!/bin/sh
# Fetch a run's server copy, whole or partial, for an organizer. It reads bytes and decodes nothing.
#
#   sh scripts/fetch-run.sh [--bucket B] [--index FILE] [--store DIR] [--local PERSIST_DIR] <run | entry> <outdir>
#
# Writes <outdir>/<entry>/ for every entry of the run (or the one entry named) and prints each path:
#   rec.gcr.zst  the entry's chunk objects, SHA-256 checked, joined in offset order, never decompressed
#   fetch.json   the entry row, "complete" and "missing", every heartbeat row, and the chunks written
# Layout and checks: PRD-07 section 7 (seam S13). Exit 1 when an object is missing or bad, the chain
# breaks or the index has a hole; a partial entry is normal and exits 0. Nothing is ever overwritten.
#
# Sources (python3 always):
#   default        psql with DATABASE_URL (a read-only role; the query runs in BEGIN READ ONLY and ends
#                  ROLLBACK, and keeps to the staging or production events by the bucket's name), and
#                  curl with CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN (read-only R2)
#   --index FILE   the index (this script's query result) from FILE instead of the database
#   --store DIR    each object from DIR/<r2 key> instead of R2
#   --local DIR    a `wrangler dev` Worker (DB_BACKEND=local, --persist-to DIR): the index from
#                  state/runs/<run>.json, objects through `npx wrangler r2 object get --local` in
#                  server/worker. Takes a run id; keeps only each entry's last heartbeat
#   --bucket B     default goalcup-traces, or goalcup-traces-staging with --local (wrangler dev keeps
#                  local R2 under preview_bucket_name)
# Credentials come from the environment only, never reach argv or output, and tracing is off.
set -eu
# Before any secret is read, and for good: `sh -x` must not echo a credential.
set +x
umask 077

bucket= index= store= persist=
while [ $# -gt 0 ]; do
  case $1 in
    --bucket) bucket=${2:?--bucket needs a bucket name}; shift ;;
    --index) index=${2:?--index needs a file}; shift ;;
    --store) store=${2:?--store needs a directory}; shift ;;
    --local) persist=${2:?--local needs the wrangler persist directory}; shift ;;
    -h | --help) sed -n '2,23p' "$0"; exit 0 ;;
    -*) echo "fetch-run: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
  shift
done
if [ $# -ne 2 ]; then
  echo "usage: sh scripts/fetch-run.sh [--bucket B] [--index FILE] [--store DIR] [--local PERSIST_DIR] <run | entry> <outdir>" >&2
  exit 2
fi
id=$1 out=$2
case $id in
  r[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f] | \
  e[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "fetch-run: $id is not a run id or an entry id (r or e, then 16 lowercase hex digits)" >&2; exit 2 ;;
esac
if [ -z "$bucket" ]; then
  if [ -n "$persist" ]; then bucket=goalcup-traces-staging; else bucket=goalcup-traces; fi
fi
case $bucket in
  '' | *[!A-Za-z0-9._-]*) echo "fetch-run: bad bucket name" >&2; exit 2 ;;
  *-staging) staging=true ;;
  *) staging=false ;;
esac
if [ -n "$persist" ]; then
  [ -d "$persist" ] || { echo "fetch-run: --local $persist is not a directory" >&2; exit 2; }
  persist=$(cd "$persist" && pwd)
  case $id in
    e*) [ -n "$index" ] || { echo "fetch-run: with --local, name the run (r...), not an entry" >&2; exit 2; } ;;
  esac
fi
if [ -z "$index" ] && [ -z "$persist" ]; then
  : "${DATABASE_URL:?set DATABASE_URL (a read-only Postgres URL), or pass --index FILE or --local DIR}"
fi
if [ -z "$store" ] && [ -z "$persist" ]; then
  : "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID, or pass --store DIR or --local DIR}" "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN, or pass --store DIR or --local DIR}"
  case $CLOUDFLARE_ACCOUNT_ID in
    '' | *[!A-Za-z0-9]*) echo "fetch-run: bad CLOUDFLARE_ACCOUNT_ID" >&2; exit 2 ;;
  esac
fi
worker_dir=$(cd "$(dirname "$0")/.." && pwd)/server/worker

mkdir -p "$out"
tmp=$(mktemp -d "$out/.fetch-run.XXXXXX") || { echo "fetch-run: cannot make a temporary directory in $out" >&2; exit 1; }
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' HUP INT TERM

# 1. The index from the database: one JSON array, an element per entry.
if [ -z "$index" ] && [ -z "$persist" ]; then
  # The URL goes into a private service file, not onto psql's command line (where `ps` shows it).
  FETCH_RUN_URL=$DATABASE_URL python3 -I - "$tmp/pg_service.conf" <<'PY'
import os, shlex, sys, urllib.parse

url = os.environ["FETCH_RUN_URL"]
kept = ("sslmode", "sslrootcert", "sslcert", "sslkey", "connect_timeout", "application_name")
pairs = {}
if url.startswith(("postgres://", "postgresql://")):
    u = urllib.parse.urlsplit(url)
    for key, value in (("host", u.hostname), ("port", u.port), ("user", u.username),
                       ("password", u.password), ("dbname", u.path.lstrip("/"))):
        if value not in (None, ""):
            pairs[key] = urllib.parse.unquote(str(value))
    for key, value in urllib.parse.parse_qsl(u.query):
        if key in kept:
            pairs[key] = value
else:  # a "host=... user=..." connection string
    for word in shlex.split(url):
        key, _, value = word.partition("=")
        pairs[key] = value
if any("\n" in v for v in pairs.values()):
    sys.exit("fetch-run: DATABASE_URL has a newline in it")
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(fd, "w") as f:
    f.write("[fetchrun]\n" + "".join(f"{k}={v}\n" for k, v in pairs.items()))
PY
  index=$tmp/index.json
  PGSERVICEFILE="$tmp/pg_service.conf" PGSERVICE=fetchrun \
    psql -X -q -At -v ON_ERROR_STOP=1 -v id="$id" -v staging="$staging" >"$index" <<'SQL'
BEGIN READ ONLY;
SELECT coalesce(jsonb_agg(jsonb_build_object(
  'run_id', e.run_id, 'entry_id', e.id, 'segment', e.segment,
  'upload_offset', e.upload_offset, 'next_seq', e.next_seq, 'head', e.head,
  'model_requests', e.model_requests, 'final_seq', e.final_seq, 'final_head', e.final_head,
  'final_at', to_char(e.final_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
  'sealed_at', to_char(e.sealed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
  'stored_bytes', e.stored_bytes, 'chunk_count', e.chunk_count,
  'heartbeats', (SELECT coalesce(jsonb_agg(jsonb_build_object(
      'received_at', to_char(h.received_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
      'seq', h.seq, 'offset_local', h.offset_local, 'final', h.final, 'recording', h.recording,
      'model_requests', h.model_requests,
      'counts', h.vitals -> 'counts', 'counts_dropped', h.vitals -> 'counts_dropped')
      ORDER BY h.received_at, h.id), '[]'::jsonb)
    FROM goal_cup_tracer.heartbeats h WHERE h.entry_id = e.id),
  'chunks', (SELECT coalesce(jsonb_agg(jsonb_build_object(
      'offset', c."offset", 'length', c.length, 'sha256', c.sha256,
      'first_seq', c.first_seq, 'last_seq', c.last_seq, 'prev', c.prev, 'head', c.head,
      'r2_key', c.r2_key,
      'received_at', to_char(c.received_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'))
      ORDER BY c."offset"), '[]'::jsonb)
    FROM goal_cup_tracer.chunks c WHERE c.entry_id = e.id)
  ) ORDER BY e.run_id, e.segment), '[]'::jsonb)
FROM goal_cup_tracer.entries e
JOIN goal_cup_tracer.runs r ON r.id = e.run_id
JOIN goal_cup_tracer.events ev ON ev.id = r.event_id
-- Staging and production share one database: keep to the side the bucket names.
WHERE (e.run_id = :'id' OR e.id = :'id') AND ev.staging = :'staging';
ROLLBACK;
SQL
fi

# 2. Check the index, fetch every object, write each entry. The R2 token, when used, reaches curl in
# its configuration on stdin; it is never in an argument or the output.
FETCH_RUN_WORKER_DIR=$worker_dir python3 -I - "$id" "$out" "$tmp" "$bucket" "$index" "$store" "$persist" <<'PY'
import hashlib, json, os, re, shutil, subprocess, sys

wanted, out, tmp, bucket, index_path, store, persist = sys.argv[1:8]
GENESIS = "0" * 64
HEX64 = re.compile(r"^[0-9a-f]{64}$")
ENTRY = re.compile(r"^e[0-9a-f]{16}$")
RUN = re.compile(r"^r[0-9a-f]{16}$")
KEY = re.compile(r"^traces/[A-Za-z0-9._/-]+$")


def say(text):
    print("fetch-run: " + text, file=sys.stderr)


class ReadFailed(Exception):
    """A read that failed for a reason other than the object being absent (refused, network)."""


def key_ok(key):
    return isinstance(key, str) and bool(KEY.match(key)) and ".." not in key.split("/") and "//" not in key


def is_int(v, low=0):
    return isinstance(v, int) and not isinstance(v, bool) and v >= low


def get_object(key, dest):
    """Fetch one object into dest. True when fetched, False when the object is absent."""
    if store:
        src = os.path.join(store, key)
        if not os.path.isfile(src):
            return False
        shutil.copyfile(src, dest)
        return True
    if persist:
        env = {k: v for k, v in os.environ.items() if not k.startswith(("CLOUDFLARE_", "DATABASE_URL"))}
        env.update(WRANGLER_SEND_METRICS="false", CI="1", NO_COLOR="1")
        r = subprocess.run(
            ["npx", "--no-install", "wrangler", "r2", "object", "get", f"{bucket}/{key}",
             "--local", "--persist-to", persist, "--file", dest],
            cwd=os.environ["FETCH_RUN_WORKER_DIR"], env=env, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=600,
        )
        said = r.stdout + r.stderr
        if r.returncode == 0 and os.path.isfile(dest):
            return True
        if os.path.exists(dest):
            os.remove(dest)
        if re.search(r"does not exist|not found|NoSuchKey", said, re.I):
            return False
        raise ReadFailed(f"wrangler could not read {key} from the local store:\n{said[-2000:]}")
    account = os.environ["CLOUDFLARE_ACCOUNT_ID"]
    url = f"https://api.cloudflare.com/client/v4/accounts/{account}/r2/buckets/{bucket}/objects/{key.replace('/', '%2F')}"
    config = (
        f'url = "{url}"\noutput = "{dest}"\n'
        f'header = "Authorization: Bearer {os.environ["CLOUDFLARE_API_TOKEN"]}"\n'
        'silent\nshow-error\nconnect-timeout = 15\nmax-time = 600\nwrite-out = "%{http_code}"\n'
    )
    r = subprocess.run(["curl", "-K", "-"], input=config, capture_output=True, text=True)
    status = r.stdout.strip()
    if r.returncode == 0 and status == "200":
        return True
    if os.path.exists(dest):
        os.remove(dest)
    if r.returncode == 0 and status == "404":
        return False
    raise ReadFailed(f"could not read {key} from bucket {bucket} (HTTP {status or 'none'})")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def local_index():
    """The run's state object in the local store (db-local.ts's shape), as the database index's shape."""
    dest = os.path.join(tmp, "state.json")
    if not get_object(f"state/runs/{wanted}.json", dest):
        sys.exit(f"fetch-run: the local store holds no run {wanted}")
    with open(dest) as f:
        run = json.load(f)
    entries = []
    for e in run.get("entries") or []:
        final = e.get("final")
        hb = e.get("last_heartbeat")
        vitals = (hb or {}).get("vitals")
        vitals = vitals if isinstance(vitals, dict) else {}
        heartbeats = [] if not hb else [{
            "received_at": hb.get("at"), "seq": hb.get("seq"), "offset_local": hb.get("offset_local"),
            "final": bool(final) and final.get("seq") == hb.get("seq") and final.get("head") == hb.get("head"),
            "recording": hb.get("recording"),
            # The local store keeps the entry's highest model_requests, not each heartbeat's.
            "model_requests": e.get("model_requests"),
            "counts": vitals.get("counts"), "counts_dropped": vitals.get("counts_dropped"),
        }]
        entries.append({
            "run_id": e.get("run_id") or run.get("run_id"), "entry_id": e.get("entry_id"),
            "segment": e.get("segment"), "upload_offset": e.get("offset"), "next_seq": e.get("next_seq"),
            "head": e.get("head"), "model_requests": e.get("model_requests"),
            "final_seq": final.get("seq") if final else None, "final_head": final.get("head") if final else None,
            "final_at": None, "sealed_at": e.get("sealed_at"),
            "stored_bytes": e.get("stored_bytes"), "chunk_count": e.get("chunk_count"),
            "heartbeats": heartbeats, "chunks": e.get("chunks") or [],
        })
    return entries


if persist and not index_path:
    entries = local_index()
else:
    with open(index_path) as f:
        entries = json.load(f)
if not isinstance(entries, list) or not entries:
    sys.exit(f"fetch-run: the index holds no entry for {wanted}")
for e in entries:
    if not isinstance(e, dict) or not isinstance(e.get("entry_id"), str) or not ENTRY.match(e["entry_id"]):
        sys.exit("fetch-run: the index names a bad entry id")
    if not RUN.match(str(e.get("run_id"))):
        sys.exit(f"fetch-run: entry {e['entry_id']} has a bad run id in the index")
    if os.path.lexists(os.path.join(out, e["entry_id"])):
        sys.exit(f"fetch-run: {os.path.join(out, e['entry_id'])} exists; nothing was overwritten or fetched")


def index_hole(chunks):
    """Why the entry's chunk rows do not run on (a hole, an overlap, a malformed row), or None."""
    offset, seq = 0, 0
    for c in chunks:
        if not (isinstance(c, dict) and is_int(c.get("offset")) and is_int(c.get("length"), 1)
                and is_int(c.get("first_seq")) and is_int(c.get("last_seq"))
                and c["first_seq"] <= c["last_seq"] and isinstance(c.get("sha256"), str)
                and HEX64.match(c["sha256"]) and isinstance(c.get("prev"), str)
                and isinstance(c.get("head"), str) and key_ok(c.get("r2_key"))):
            return "a malformed chunk row"
        if c["offset"] != offset:
            return f"the chunk at byte {c['offset']} follows one that ends at {offset}"
        if c["first_seq"] != seq:
            return f"the chunk at byte {c['offset']} starts at seq {c['first_seq']}, not {seq}"
        offset, seq = c["offset"] + c["length"], c["last_seq"] + 1
    return None


failed = False
for e in entries:
    entry = e["entry_id"]
    chunks = e.get("chunks") or []  # in offset order, as the query and the local store give them
    hole = index_hole(chunks)
    if hole:
        say(f"entry {entry}: the index has a hole ({hole}); nothing was written for it")
        failed = True
        continue
    build = os.path.join(tmp, "build-" + entry)
    os.mkdir(build)
    part = os.path.join(tmp, "object")
    written, why = [], None
    offset, seq, head = 0, 0, GENESIS
    with open(os.path.join(build, "rec.gcr.zst"), "wb") as rec:
        for c in chunks:
            if c["prev"] != head:
                why = {"why": "chain_break", "offset": c["offset"], "r2_key": c["r2_key"]}
                say(f"entry {entry}: the chunk at byte {c['offset']} does not follow the previous head; the copy stops before it")
                break
            try:
                present = get_object(c["r2_key"], part)
            except ReadFailed as err:
                say(f"entry {entry}: {err}; nothing was written for it")
                sys.exit(1)
            if not present or sha256_file(part) != c["sha256"]:
                why = {"why": "object_bad", "offset": c["offset"], "r2_key": c["r2_key"]}
                say(f"entry {entry}: {c['r2_key']} is " + ("missing" if not present else "not the bytes its SHA-256 names")
                    + "; the copy stops before it")
                break
            size = os.path.getsize(part)
            with open(part, "rb") as src:
                shutil.copyfileobj(src, rec, 1 << 20)
            os.remove(part)
            written.append({
                "offset": c["offset"], "length": c["length"], "bytes": size, "sha256": c["sha256"],
                "first_seq": c["first_seq"], "last_seq": c["last_seq"], "prev": c["prev"], "head": c["head"],
                "r2_key": c["r2_key"], "received_at": c.get("received_at"),
            })
            offset, seq, head = c["offset"] + c["length"], c["last_seq"] + 1, c["head"]
    if os.path.exists(part):
        os.remove(part)
    upload_offset, next_seq, entry_head = e.get("upload_offset"), e.get("next_seq"), e.get("head")
    if why is None and (offset, seq, head) != (upload_offset, next_seq, entry_head):
        why = {"why": "index_mismatch", "offset": offset}
        say(f"entry {entry}: the chunks end at byte {offset} and seq {seq}, the entry row at byte {upload_offset} and seq {next_seq}")
    final = None
    if e.get("final_seq") is not None:
        final = {"seq": e["final_seq"], "head": e.get("final_head"), "at": e.get("final_at")}
    complete = (why is None and final is not None and is_int(next_seq)
                and next_seq - 1 == final["seq"] and entry_head == final["head"])
    missing = None
    if not complete:
        missing = {"from_offset": offset, "from_seq": seq, "to_seq": final["seq"] if final else None}
        if why is not None:
            missing.update(why)
            failed = True
    heartbeats = [
        {k: h.get(k) for k in ("received_at", "seq", "offset_local", "final", "recording", "model_requests", "counts", "counts_dropped")}
        for h in (e.get("heartbeats") or []) if isinstance(h, dict)
    ]
    doc = {
        "v": 1, "run_id": e["run_id"], "entry_id": entry, "segment": e.get("segment"),
        "upload_offset": upload_offset, "next_seq": next_seq, "head": entry_head,
        "model_requests": e.get("model_requests"), "final": final, "sealed_at": e.get("sealed_at"),
        "complete": complete, "missing": missing,
        "last_heartbeat": heartbeats[-1] if heartbeats else None, "heartbeats": heartbeats,
        "stored_bytes": e.get("stored_bytes"), "chunk_count": e.get("chunk_count"), "chunks": written,
    }
    with open(os.path.join(build, "fetch.json"), "w") as f:
        json.dump(doc, f, separators=(",", ":"))
        f.write("\n")
    dest = os.path.join(out, entry)
    os.rename(build, dest)
    print(dest, flush=True)
sys.exit(1 if failed else 0)
PY
