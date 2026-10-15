#!/bin/sh
# Rebuild a run's trace from the server's copy, for an organizer, so `goalcup-verify` can judge it.
#
#   sh scripts/fetch-run.sh [--bucket NAME] [--index FILE] [--store DIR] <run-id | entry-id> <outdir>
#
# The chunk index in the database says which chunks make up each entry (an R2 listing could show an
# orphan object that lost a race, so it is never used). For every entry of the run, or for the one
# entry named, this writes the directory <outdir>/<entry-id>/ holding
#   chain.jsonl    the chunks, decompressed and joined in offset order
#   acks.jsonl     the server's signed acknowledgement of each chunk, in offset order
#   receipt.json   the signed start receipt, when the entry has one
#   seal.json, baseline.json   the documents the seal call stored, when the entry is sealed
# then prints each directory on stdout. It checks every chunk: the downloaded bytes match the
# index's SHA-256, they decompress to exactly the indexed length, and the offsets run on from 0 with
# no gap. A directory appears only when its entry is complete; a failure leaves nothing behind.
# It never overwrites: an existing <outdir>/<entry-id> is refused.
#
# Judge a result with
#   goalcup-verify --capture-integrity <outdir>/<entry-id>
# (add --dev-key <hex> for a staging or test run, whose receipt is not signed by the production key).
#
# Needs: zstd, python3, and either
#   psql and curl, with in the environment
#     DATABASE_URL           a Postgres URL; give it a read-only role. The query runs inside
#                            BEGIN READ ONLY, so it cannot write whatever the role may do.
#     CLOUDFLARE_ACCOUNT_ID  and CLOUDFLARE_API_TOKEN (an R2 token with read-only access)
#   or the two options below, which need no credentials:
#     --index FILE   read the chunk index from FILE (the JSON this script's query returns) instead
#                    of the database
#     --store DIR    read each object from DIR/<r2 key> instead of R2 (a mirror of the bucket)
# --bucket NAME is the R2 bucket (default goalcup-traces; staging is goalcup-traces-staging).
# Credentials are read from the environment only. They are never printed or passed on a command
# line, and the script only ever reads (a SELECT, and GET requests).
set -eu

bucket=goalcup-traces index= store=
while [ $# -gt 0 ]; do
  case $1 in
    --bucket) bucket=${2:?--bucket needs a bucket name}; shift ;;
    --index) index=${2:?--index needs a file}; shift ;;
    --store) store=${2:?--store needs a directory}; shift ;;
    -h | --help) sed -n '2,33p' "$0"; exit 0 ;;
    -*) echo "fetch-run: unknown option $1" >&2; exit 2 ;;
    *) break ;;
  esac
  shift
done
if [ $# -ne 2 ]; then
  echo "usage: sh scripts/fetch-run.sh [--bucket NAME] [--index FILE] [--store DIR] <run-id | entry-id> <outdir>" >&2
  exit 2
fi
id=$1 out=$2
case $id in
  r[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f] | \
  e[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "fetch-run: $id is not a run id or an entry id (r or e, then 16 lowercase hex digits)" >&2; exit 2 ;;
esac
case $bucket in
  '' | *[!A-Za-z0-9._-]*) echo "fetch-run: bad bucket name" >&2; exit 2 ;;
esac
command -v zstd >/dev/null 2>&1 || { echo "fetch-run: zstd is needed" >&2; exit 2; }
command -v python3 >/dev/null 2>&1 || { echo "fetch-run: python3 is needed" >&2; exit 2; }
if [ -z "$index" ]; then
  : "${DATABASE_URL:?set DATABASE_URL (a read-only Postgres URL), or pass --index FILE}"
  command -v psql >/dev/null 2>&1 || { echo "fetch-run: psql is needed" >&2; exit 2; }
fi
if [ -z "$store" ]; then
  : "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID, or pass --store DIR}" "${CLOUDFLARE_API_TOKEN:?set CLOUDFLARE_API_TOKEN, or pass --store DIR}"
  command -v curl >/dev/null 2>&1 || { echo "fetch-run: curl is needed" >&2; exit 2; }
  case $CLOUDFLARE_ACCOUNT_ID in
    '' | *[!A-Za-z0-9]*) echo "fetch-run: bad CLOUDFLARE_ACCOUNT_ID" >&2; exit 2 ;;
  esac
fi

mkdir -p "$out"
tmp=$(mktemp -d "$out/.fetch-run.XXXXXX") && trap 'rm -rf "$tmp"' EXIT

# 1. The chunk index: one JSON array, an element per entry, each with its chunks in offset order.
if [ -n "$index" ]; then
  cp "$index" "$tmp/index.json"
else
  psql "$DATABASE_URL" -X -q -At -v ON_ERROR_STOP=1 -v id="$id" >"$tmp/index.json" <<'SQL'
BEGIN READ ONLY;
SELECT coalesce(jsonb_agg(jsonb_build_object(
  'run_id', e.run_id, 'entry_id', e.id, 'segment', e.segment, 'receipt', e.receipt,
  'seal_key', e.seal_key, 'seal_sha256', e.seal_sha256,
  'baseline_key', e.baseline_key, 'baseline_sha256', e.baseline_sha256,
  'chunks', (SELECT coalesce(jsonb_agg(jsonb_build_object(
      'offset', c."offset", 'length', c.length, 'sha256', c.sha256, 'first_seq', c.first_seq,
      'last_seq', c.last_seq, 'head', c.head, 'r2_key', c.r2_key, 'ack', c.ack)
      ORDER BY c."offset"), '[]'::jsonb)
    FROM goal_cup_tracer.chunks c WHERE c.entry_id = e.id)
  ) ORDER BY e.run_id, e.segment), '[]'::jsonb)
FROM goal_cup_tracer.entries e
WHERE e.run_id = :'id' OR e.id = :'id';
ROLLBACK;
SQL
fi

# 2. Check the index and turn it into a plan. Lines: "E entry seal_key seal_sha baseline_key
# baseline_sha" then "C entry offset length sha256 r2_key" for each chunk; "-" is absent. Keys are
# held to a plain alphabet, so they are safe in a path and a URL. The receipt and the acks are
# written here, from the index.
python3 - "$tmp/index.json" "$tmp/plan" "$tmp/meta" "$id" <<'PY'
import json, os, re, sys

index_path, plan_path, meta_dir, wanted = sys.argv[1:5]
with open(index_path) as f:
    entries = json.load(f)
if not isinstance(entries, list) or not entries:
    sys.exit(f"fetch-run: the database holds no entry for {wanted}")

HEX64 = re.compile(r"^[0-9a-f]{64}$")
ENTRY = re.compile(r"^e[0-9a-f]{16}$")
KEY = re.compile(r"^traces/[A-Za-z0-9._/-]+$")


def key_ok(key):
    return isinstance(key, str) and KEY.match(key) and ".." not in key.split("/") and "//" not in key


def dash(value):
    return value if value else "-"


plan = []
for e in entries:
    entry = e.get("entry_id")
    if not isinstance(entry, str) or not ENTRY.match(entry):
        sys.exit("fetch-run: the index names a bad entry id")
    chunks = e.get("chunks") or []
    if not chunks:
        sys.exit(f"fetch-run: entry {entry} has no chunks in the index, so there is no chain to rebuild")
    seal_key, baseline_key = e.get("seal_key"), e.get("baseline_key")
    for key, sha in ((seal_key, e.get("seal_sha256")), (baseline_key, e.get("baseline_sha256"))):
        if key is not None and (not key_ok(key) or not (isinstance(sha, str) and HEX64.match(sha))):
            sys.exit(f"fetch-run: entry {entry} has a bad document key or digest in the index")
    plan.append(f"E {entry} {dash(seal_key)} {dash(e.get('seal_sha256'))} {dash(baseline_key)} {dash(e.get('baseline_sha256'))}")
    expected = 0
    acks = []
    for c in chunks:
        offset, length = c.get("offset"), c.get("length")
        ok = (
            isinstance(offset, int) and isinstance(length, int) and length > 0
            and isinstance(c.get("sha256"), str) and HEX64.match(c["sha256"])
            and key_ok(c.get("r2_key")) and isinstance(c.get("ack"), dict)
        )
        if not ok:
            sys.exit(f"fetch-run: entry {entry} has a malformed chunk row in the index")
        if offset != expected:
            sys.exit(
                f"fetch-run: entry {entry}: the next chunk starts at byte {offset} but the one before ends at {expected} "
                "(the index has a gap or an overlap); nothing was written"
            )
        expected = offset + length
        plan.append(f"C {entry} {offset} {length} {c['sha256']} {c['r2_key']}")
        acks.append(json.dumps(c["ack"], separators=(",", ":")))
    d = os.path.join(meta_dir, entry)
    os.makedirs(d)
    with open(os.path.join(d, "acks.jsonl"), "w") as f:
        f.write("\n".join(acks) + "\n")
    if e.get("receipt") is not None:
        with open(os.path.join(d, "receipt.json"), "w") as f:
            f.write(json.dumps(e["receipt"], separators=(",", ":")) + "\n")
    with open(os.path.join(d, "bytes"), "w") as f:
        f.write(str(expected))
with open(plan_path, "w") as f:
    f.write("\n".join(plan) + "\n")
PY

# 3. One object into $2, from the local store or from R2. R2's REST API takes the key percent-encoded.
# The credential reaches curl through its configuration on stdin, never its command line.
get_object() {
  if [ -n "$store" ]; then
    [ -f "$store/$1" ] || { echo "fetch-run: $1 is not in $store" >&2; return 1; }
    cp "$store/$1" "$2"
  else
    enc=$(printf '%s' "$1" | sed 's|/|%2F|g')
    url="https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/r2/buckets/$bucket/objects/$enc"
    {
      printf 'url = "%s"\noutput = "%s"\n' "$url" "$2"
      printf 'header = "Authorization: Bearer %s"\n' "$CLOUDFLARE_API_TOKEN"
      printf 'fail\nsilent\nshow-error\n'
    } | curl -K - || { echo "fetch-run: could not read $1 from bucket $bucket" >&2; return 1; }
  fi
}

sha256_of() {
  python3 -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$1"
}

# 4. Build each entry's directory inside $tmp. Only when every entry is whole are they moved into
# $out, so a failure anywhere leaves nothing there.
fetch_document() { # entry name key sha256
  [ "$3" = "-" ] && {
    echo "fetch-run: entry $1 has no $2.json (it was not sealed on the server); the verifier will call it incomplete" >&2
    return 0
  }
  get_object "$3" "$tmp/build-$1/$2.json" || return 1
  [ "$(sha256_of "$tmp/build-$1/$2.json")" = "$4" ] || {
    echo "fetch-run: $3 does not match the index's SHA-256" >&2
    return 1
  }
}

entries=
while read -r kind a b c d e f; do
  case $kind in
    E)
      entry=$a
      if [ -e "$out/$entry" ]; then
        echo "fetch-run: $out/$entry exists; nothing was overwritten" >&2
        exit 1
      fi
      mkdir "$tmp/build-$entry"
      : >"$tmp/build-$entry/chain.jsonl"
      cp "$tmp/meta/$entry/acks.jsonl" "$tmp/build-$entry/acks.jsonl"
      if [ -f "$tmp/meta/$entry/receipt.json" ]; then
        cp "$tmp/meta/$entry/receipt.json" "$tmp/build-$entry/receipt.json"
      fi
      fetch_document "$entry" seal "$b" "$c" || exit 1
      fetch_document "$entry" baseline "$d" "$e" || exit 1
      entries="$entries $entry"
      ;;
    C)
      entry=$a offset=$b length=$c sha=$d key=$e
      get_object "$key" "$tmp/chunk" || exit 1
      [ "$(sha256_of "$tmp/chunk")" = "$sha" ] || {
        echo "fetch-run: $key does not match the index's SHA-256 (entry $entry, byte $offset)" >&2
        exit 1
      }
      # Decompress at most one byte past the stated length, so an oversized chunk is caught, not kept.
      zstd -dcq "$tmp/chunk" | head -c "$((length + 1))" >"$tmp/part" || true
      got=$(wc -c <"$tmp/part" | tr -d ' ')
      [ "$got" = "$length" ] || {
        echo "fetch-run: entry $entry, byte $offset: the chunk holds $got bytes (or is not zstd), the index says $length" >&2
        exit 1
      }
      cat "$tmp/part" >>"$tmp/build-$entry/chain.jsonl"
      rm -f "$tmp/chunk" "$tmp/part"
      ;;
  esac
done <"$tmp/plan"

for entry in $entries; do
  [ "$(wc -c <"$tmp/build-$entry/chain.jsonl" | tr -d ' ')" = "$(cat "$tmp/meta/$entry/bytes")" ] || {
    echo "fetch-run: entry $entry came out the wrong length" >&2
    exit 1
  }
done
for entry in $entries; do
  mv "$tmp/build-$entry" "$out/$entry"
  printf '%s\n' "$out/$entry"
done
echo "fetch-run: now run: goalcup-verify --capture-integrity <dir> (add --dev-key <hex> for a staging or test run)" >&2
