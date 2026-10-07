#!/bin/sh
# Regenerate the two connector pin tables from the shipped connectors. Run it in the same
# commit as any change under connectors/ (Global Constraint 8).
#
# Two tables, in two places, because two different questions are asked at two different
# times — and because one of the two askers may not link the verifier:
#
#   CONNECTOR_MANIFEST_SHA256S — SHA-256 of `connectors/<id>/manifest.toml`'s bytes (the V1
#     manifest). This is the digest the daemon puts on the arming receipt's `manifest_sha256`
#     claim and that the verifier recomputes from the `manifest.toml` a trace carries, so V13
#     can compare a pin against bytes it holds in its hand. A trace does not carry the rest of
#     the connector directory; this digest is the most a post-hoc verifier can recompute.
#     Written as crates/goalcup-verifier/pins/connector-manifests.txt. Only folders that hold
#     a manifest.toml are in it; the verifier embeds it through the protocol pin parser.
#
#   crates/goalcup-protocol/pins/connector-dirs.txt — SHA-256 of the whole `connectors/<id>/`
#     directory, the files under `files/` included, one `<id> <sha256>` line per connector
#     folder, sorted by id. DATA, not Rust (harness contract v1, S2): the protocol crate
#     reads it at build time (`crates/goalcup-protocol/src/pins.rs`, a `const fn` parser that
#     refuses a malformed file), so a harness PR needs no `.rs` edit. It lives in the protocol
#     crate and NOT beside the table above, because the daemon reads it and the daemon must
#     never depend on goalcup-verifier (D3): the daemon hashes its own connectors directory at
#     workspace creation, compares it here, and puts the value on the start receipt, where V13
#     compares it against this same table a week later. A directory carrying the genuine
#     manifest beside an edited seed reads as pinned under the first table and fails under
#     this one.
#
# THE DIRECTORY DIGEST'S EXACT BYTE LAYOUT. For every regular file under the directory,
# recursively, in ascending order of the BYTES of its path relative to the directory with
# `/` separators, the hashed stream carries
#
#     <u64 be: path byte count> <relative path bytes> <u64 be: file byte count> <file bytes>
#
# and the digest is that stream's SHA-256 in lowercase hex. BOTH halves are length-prefixed
# and nothing is separated by a sentinel. The first spelling of this layout put a NUL after
# the path and a NUL after the contents, which is injective only while no file body holds a
# NUL — and a connector may ship a binary asset. With NUL separators a file `a` holding
# `b\0c` and the pair `a` holding `b`, `c` holding nothing hash one stream. A byte count
# cannot be forged from inside the bytes it counts. Big-endian and fixed-width, so this
# script and the Rust walk write the same eight bytes on any host.
#
# `goalcup_protocol::manifest::connector_dir_sha256` computes the same stream in Rust, and
# `crates/goalcup-verifier/tests/checks_v13.rs` runs this script's `--print` mode and
# compares the two over every shipped connector — so the layout is written down twice
# and measured once.
#
# THE TABLES ARE KEYED BY THE DECLARED id, never by a directory's basename. The daemon, the
# CLI and V13 all look a pin up by the id the manifest DECLARES; a table keyed by the
# basename would silently pin nothing for a directory whose name and declared id differ,
# and the lookup would return None — an unpinned connector — rather than a mismatch. So the
# `id = "..."` line is parsed out of each manifest.toml and that is the key.
#
# ROW ORDER IS THE C-LOCALE ORDER: `LC_ALL=C` is exported below, so the glob, the sort inside
# the directory walk and the sort of the pin lines (by id) compare bytes rather than letting a
# caller's collation reorder `_`, `-` and case. Without it the same connectors produce a
# different table on two machines and the diff never settles.
#
# A symlink, a fifo, a socket or a device node anywhere under a connector directory is
# refused rather than skipped or followed, exactly as the Rust function refuses it: a
# skipped entry is a byte the digest does not cover, and a followed symlink pins bytes that
# live outside the directory. Names carrying a newline or a backslash are refused too —
# this script reads its file list a line at a time, and a name it cannot read back is a
# name whose digest would silently differ from the Rust one.
#
# The pins are data: every digest below is recomputed from the bytes it pins. Both tables are
# written whole to their data files. Nothing here is ever typed by hand. The script is
# idempotent, so
#
#     sh scripts/pin-manifests.sh && git diff --exit-code
#
# is what proves a commit's pins match its connectors.
#
# Usage:
#     sh scripts/pin-manifests.sh            # rewrite both tables
#     sh scripts/pin-manifests.sh --print    # print "<id> <manifest sha> <dir sha>" lines
#
# `--print` writes nothing, and is what the Rust agreement test reads. It prints one line
# per connector folder. The manifest field is `-` when the folder has no legacy V1 manifest,
# while the directory field always carries the folder digest.
set -eu
cd "$(dirname "$0")/.."

# Byte order for the glob, for `sort`, and for every `case` glob below. Exported rather
# than prefixed onto one command, because the row order of a generated table must not
# depend on the collation of whoever ran the script.
LC_ALL=C
export LC_ALL

mode=rewrite
if [ $# -gt 0 ]; then
  case $1 in
    --print) mode=print ;;
    *)
      echo "pin-manifests: unknown argument '$1'; expected --print or nothing" >&2
      exit 2
      ;;
  esac
fi

# Pick the digest tool this machine has, and refuse to run without one. The previous
# version hard-coded `shasum -a 256 "$m" | cut -d' ' -f1`, whose exit status is `cut`'s:
# on a machine carrying only `sha256sum` the pipeline "succeeded" with empty output and
# the script generated empty pins — a build that pins nothing, which is exactly
# the state the constant exists to make impossible.
if command -v shasum >/dev/null 2>&1; then
  sha256_line() { shasum -a 256 "$1"; }
elif command -v sha256sum >/dev/null 2>&1; then
  sha256_line() { sha256sum "$1"; }
else
  echo "pin-manifests: need shasum or sha256sum on PATH, found neither" >&2
  exit 1
fi

# A private temporary DIRECTORY rather than fixed /tmp paths: two checkouts regenerating
# pins at once must not overwrite each other's work, and a run that fails must not leave a
# half-written block behind for the next.
work=$(mktemp -d "${TMPDIR:-/tmp}/goalcup-pins.XXXXXX")
trap 'rm -rf "$work"' EXIT

# `<digest>  <path>` split with a parameter expansion rather than a pipe, so `set -e`
# still sees the digest tool's own exit status.
digest_of_file() {
  line=$(sha256_line "$1")
  printf '%s' "${line%% *}"
}

# The eight bytes that prefix each half of a file's record: the count, big-endian.
#
# Written as octal escapes because that is the one way POSIX `printf` emits an arbitrary
# byte, NUL included. `$(($1))` normalises `wc -c`'s leading blanks on the BSD tools.
put_u64be() {
  n=$(($1))
  esc=''
  shift_by=56
  while [ "$shift_by" -ge 0 ]; do
    esc="$esc\\$(printf '%03o' "$(((n >> shift_by) & 255))")"
    shift_by=$((shift_by - 8))
  done
  # The escapes ARE the format string here; there is no `%` in it by construction.
  printf "$esc"
}

# The id a connector manifest DECLARES, which is the key every lookup uses.
#
# The top-level `id = "..."`, and only that one: `[human_rule]` carries an `id` of its
# own (and a recorded V1 manifest's `[[human_rule.exclusions]]` entries carry one each), so the first match in file order is taken and a manifest whose
# top-level id is not its first `id =` line is a manifest this script refuses rather than
# mis-keys. An id that is empty, or that is not the `[a-z0-9-]` shape a connector id has,
# is refused too — it would create an invalid lookup key in generated pin data.
declared_id() {
  id=$(sed -n 's/^id[[:space:]]*=[[:space:]]*"\([^"]*\)".*$/\1/p' "$1" | head -n 1)
  case $id in
    "" | *[!a-z0-9-]*)
      echo "pin-manifests: $1: no top-level id = \"...\" line, or an id that is not" >&2
      echo "  lowercase letters, digits and hyphens: '$id'" >&2
      exit 1
      ;;
  esac
  printf '%s' "$id"
}

# The id a connector FOLDER declares: its harness.toml's, else its manifest.toml's. Where both
# exist they must agree, or one pin table would be keyed by an id the other does not carry.
folder_id() {
  fid=''
  if [ -f "$1/harness.toml" ]; then
    fid=$(declared_id "$1/harness.toml")
    if [ -f "$1/manifest.toml" ]; then
      mid=$(declared_id "$1/manifest.toml")
      if [ "$mid" != "$fid" ]; then
        echo "pin-manifests: $1: harness.toml declares '$fid' and manifest.toml '$mid'" >&2
        exit 1
      fi
    fi
  elif [ -f "$1/manifest.toml" ]; then
    fid=$(declared_id "$1/manifest.toml")
  else
    echo "pin-manifests: $1 holds neither harness.toml nor manifest.toml" >&2
    exit 1
  fi
  printf '%s' "$fid"
}

# The directory digest, per the byte layout in this file's header.
digest_of_dir() {
  d=$1
  list=$work/list
  blob=$work/blob

  # Anything that is neither a regular file nor a directory — symlinks included, because
  # `find` without `-L` does not follow them and a symlink is `-type l`, not `-type f`.
  irregular=$(find "$d" ! -type f ! -type d)
  if [ -n "$irregular" ]; then
    echo "pin-manifests: $d holds entries that are not regular files or directories, and" >&2
    echo "  a connector directory is pinned by its own bytes:" >&2
    echo "$irregular" >&2
    exit 1
  fi

  # Relative paths, `./` stripped, in byte order. `LC_ALL=C` is what makes `sort` compare
  # bytes rather than letting the caller's collation reorder `_`, `-` and case.
  (cd "$d" && find . -type f -print) | sed 's|^\./||' | LC_ALL=C sort >"$list"

  # A name carrying a newline would arrive as two lines and hash as two files. Counted
  # with `-exec` rather than by reading the list, so the count is `find`'s own and not
  # this script's line-based reading of it.
  found=$(find "$d" -type f -exec printf 'x\n' \; | wc -l)
  listed=$(wc -l <"$list")
  if [ "$found" -ne "$listed" ]; then
    echo "pin-manifests: $d: $found files, $listed lines — a name carries a newline" >&2
    exit 1
  fi

  : >"$blob"
  while IFS= read -r rel; do
    case $rel in
      *\\*)
        echo "pin-manifests: $d/$rel: a backslash in a name this script reads by line" >&2
        exit 1
        ;;
    esac
    put_u64be "$(printf '%s' "$rel" | wc -c)" >>"$blob"
    printf '%s' "$rel" >>"$blob"
    put_u64be "$(wc -c <"$d/$rel")" >>"$blob"
    cat "$d/$rel" >>"$blob"
  done <"$list"

  digest_of_file "$blob"
}

# `^[0-9a-f]{64}$`, spelled with a length test and a glob so the check itself needs nothing
# on PATH. Nothing downstream would notice an empty or truncated digest, so the script
# refuses it before writing either data file.
check_digest() {
  case $1 in
    *[!0-9a-f]* | "") ok=no ;;
    *) [ ${#1} -eq 64 ] && ok=yes || ok=no ;;
  esac
  if [ "$ok" != yes ]; then
    echo "pin-manifests: $2: expected a 64-character hex digest, got '$1'" >&2
    exit 1
  fi
}

# Every pin table is keyed by the id the connector declares. Two directories declaring one
# id would emit two rows for one lookup key, and a first-match lookup would silently choose
# one rulebook. Refuse before either --print or regeneration writes such a table.
ensure_declared_ids_are_unique() {
  seen=$work/declared-ids
  : >"$seen"
  for d in connectors/*/; do
    d=${d%/}
    id=$(folder_id "$d")
    if grep -F -x -- "$id" "$seen" >/dev/null 2>&1; then
      echo "pin-manifests: duplicate declared connector id '$id' (including $d)" >&2
      exit 1
    fi
    printf '%s\n' "$id" >>"$seen"
  done
}

ensure_declared_ids_are_unique

if [ "$mode" = print ]; then
  for dir in connectors/*/; do
    dir=${dir%/}
    id=$(folder_id "$dir")
    m=$dir/manifest.toml
    if [ -f "$m" ]; then
      file_digest=$(digest_of_file "$m")
      check_digest "$file_digest" "$m"
    else
      file_digest=-
    fi
    dir_digest=$(digest_of_dir "$dir")
    check_digest "$dir_digest" "$dir"
    printf '%s %s %s\n' "$id" "$file_digest" "$dir_digest"
  done
  exit 0
fi

# The manifest pins, one `<id> <sha256>` line per folder that holds a V1 manifest.toml.
emit_manifest_pins() {
  for m in connectors/*/manifest.toml; do
    dir=$(dirname "$m")
    id=$(folder_id "$dir")
    digest=$(digest_of_file "$m")
    check_digest "$digest" "$m"
    printf '%s %s\n' "$id" "$digest"
  done | LC_ALL=C sort
}

# The directory pins, one `<id> <sha256>` line per connector folder, in id order. Sorting the
# whole line orders by id: the id is the line's prefix, and the space after it (0x20) is below
# every character an id may hold (`[a-z0-9-]`), so a shorter id sorts before its extensions,
# exactly as pins.rs compares ids.
emit_dir_pins() {
  for d in connectors/*/; do
    d=${d%/}
    id=$(folder_id "$d")
    digest=$(digest_of_dir "$d")
    check_digest "$digest" "$d"
    printf '%s %s\n' "$id" "$digest"
  done | LC_ALL=C sort
}

manifest_pins_file=crates/goalcup-verifier/pins/connector-manifests.txt
dir_pins_file=crates/goalcup-protocol/pins/connector-dirs.txt

{
  echo '# Connector manifest pins: `<id> <sha256 of connectors/<id>/manifest.toml>`, sorted by id.'
  echo '# Written by `sh scripts/pin-manifests.sh`; never edited by hand. Embedded by'
  echo '# crates/goalcup-verifier/src/lib.rs through the protocol pin parser.'
  emit_manifest_pins
} >"$work/manifest-pins"
mkdir -p "$(dirname "$manifest_pins_file")"
cp "$work/manifest-pins" "$manifest_pins_file"

{
  echo '# Connector directory pins: `<id> <sha256 of connectors/<id>/>`, one per connector, sorted by id.'
  echo '# Written by `sh scripts/pin-manifests.sh`; never edited by hand. Read at build time by'
  echo '# crates/goalcup-protocol/src/pins.rs, which refuses a malformed file (the crate does not build).'
  emit_dir_pins
} >"$work/dir-pins"
mkdir -p "$(dirname "$dir_pins_file")"
cp "$work/dir-pins" "$dir_pins_file"
