#!/bin/sh
# Run every local check for one connector and print one PASS/FAIL/SKIPPED line per step.
#
# Usage:
#     sh scripts/check-connector.sh [--allow-no-kit] <connector-id>
#
# Steps, in order (all run, so one report shows every problem):
#   1. files: no template placeholder is left: no `<...>` outside comments in
#      connectors/<id>/harness.toml. A headless connector also needs its kit case
#      crates/goalcup-cli/tests/fixtures/harness-kit/<id>/ (named after the connector id), with
#      harness.toml, stand-in.py and expect.toml, the same rule on its two TOML files and no `<`
#      in the stand-in's EDIT block. A chat-mode connector (`interactive = true`) needs no kit
#      case: the connector contract runs every connector through one stand-in.
#      A missing connectors/<id>/harness.toml is a usage error (exit 2) before anything runs.
#      Also: when the connector's `program` is on PATH and is a `node` script, the manifest must
#      set NODE_USE_ENV_PROXY = "1" (D59), or the run records no model traffic. The kit's
#      stand-in sends no traffic, so this is the only local sign of that; a compiled Node or
#      Bun binary is not detected, and only a real run (the practice run) shows it.
#   2. pins: `sh scripts/pin-manifests.sh`, then the pin diff against HEAD is printed. It fails
#      when the diff touches another connector's line; a new or changed line for <id> is
#      expected and must be committed with the connector.
#   3. schema: `cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::`.
#   4. goldens: the CLI and daemon `harness_contract_golden` tests, without regeneration (an
#      exported GOALCUP_REGEN_HARNESS_GOLDEN is unset first). On a failure the regeneration
#      command is printed; review its diff before committing. A new connector adds only its
#      own files, <id>.json and <id>.daemon.json; no shared golden names a connector. Off macOS
#      the daemon's live golden (<id>.daemon.json) compiles to nothing, so it is SKIPPED (not a
#      failure); a maintainer checks it on a Mac.
#   5. kit (macOS and Linux): `cargo build`, then `cargo test -p goalcup-cli --test it
#      harness_contract_kit::`, then the connector contract: its static half
#      (`--test it connector_contract::`) and its dynamic half (`--test full connector_contract::`,
#      one participant-shaped run of every connector folder). On any other system the kit
#      compiles to zero tests, so the step is SKIPPED and the script exits non-zero unless
#      --allow-no-kit is given.
#
# It checks a new connector. The shipped headless connector (hermes-cli) predates the kit and
# has no case of its own, so step 1 fails for it; its regression checks are the schema and
# golden steps (3 and 4), which the gate also runs.
#
# Exit status: 0 when every step passed (or only the kit was skipped with --allow-no-kit),
# 1 when a step failed or the kit was skipped without --allow-no-kit, 2 on a usage error.
# Cargo honours CARGO_TARGET_DIR and the rest of its environment as usual.
set -u
# The script's own path, made absolute before the `cd` so `--help` can still read the header.
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P) || exit 2
self=$script_dir/$(basename "$0")
CDPATH='' cd -- "$(dirname -- "$0")/.." || exit 2
# "Without regeneration" must hold even when the caller's shell still exports the variable
# (the skill tells contributors to set it for one command): with it set, the golden tests
# rewrite a differing golden and pass, so a broken golden would be reported as PASS.
unset GOALCUP_REGEN_HARNESS_GOLDEN

usage() {
  echo "usage: sh scripts/check-connector.sh [--allow-no-kit] <connector-id>" >&2
  exit 2
}

allow_no_kit=no
id=''
for arg in "$@"; do
  case $arg in
    --allow-no-kit) allow_no_kit=yes ;;
    -h | --help)
      # The comment block after the shebang, up to the first line that is not a comment.
      awk 'NR == 1 { next } !/^#/ { exit } { print }' "$self" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*)
      echo "check-connector: unknown option '$arg'" >&2
      usage
      ;;
    *)
      if [ -n "$id" ]; then
        echo "check-connector: one connector id only (got '$id' and '$arg')" >&2
        usage
      fi
      id=$arg
      ;;
  esac
done
[ -n "$id" ] || usage
# The schema's id grammar: ^[a-z][a-z0-9-]{1,47}$.
case $id in
  [a-z]*) ;;
  *) echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2; usage ;;
esac
case $id in
  *[!a-z0-9-]*) echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2
    usage ;;
esac
if [ ${#id} -lt 2 ] || [ ${#id} -gt 48 ]; then
  echo "check-connector: '$id' is not a connector id ([a-z][a-z0-9-]{1,47})" >&2
  usage
fi

if [ ! -f "connectors/$id/harness.toml" ]; then
  echo "check-connector: no connectors/$id/harness.toml; ids here:" \
    "$(ls connectors 2>/dev/null | tr '\n' ' ')" >&2
  exit 2
fi

# One target directory per checkout: cargo does not put the checkout path in a workspace
# crate's build hash, so a CARGO_TARGET_DIR shared with another checkout can run that
# checkout's test binaries, which then read and write the other checkout's files.
# Cargo resolves a relative CARGO_TARGET_DIR against its working directory, which is the repo
# root here. A target-dir set in a cargo config file is not detected.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  target=$CARGO_TARGET_DIR
  case $target in
    /*) ;;
    *) target=$PWD/$target ;;
  esac
  # Resolve symlinks and `..` when the directory exists; a missing one with `..` warns.
  if [ -d "$target" ]; then
    target=$(cd "$target" && pwd -P)
  fi
  here=$(pwd -P)
  case $target in
    */../* | */..) inside=no ;;
    "$PWD" | "$PWD"/* | "$here" | "$here"/*) inside=yes ;;
    *) inside=no ;;
  esac
  case $inside in
    yes) ;;
    *)
      echo "check-connector: WARNING: CARGO_TARGET_DIR=$CARGO_TARGET_DIR is outside this checkout;"
      echo "  if another checkout or worktree uses it too, results may come from that checkout"
      ;;
  esac
fi

passed=0
failed=0
skipped=0
summary=''
note() { summary="$summary
$1"; }
pass() { passed=$((passed + 1)); note "check-connector: PASS $1"; echo "== PASS $1"; }
fail() { failed=$((failed + 1)); note "check-connector: FAIL $1"; echo "== FAIL $1"; }
step() { echo; echo "== $1"; }

# 1. Files.
kit=crates/goalcup-cli/tests/fixtures/harness-kit/$id
# Chat mode is `interactive = true` in the connector's [capture] table.
if grep -Eq '^[[:space:]]*interactive[[:space:]]*=[[:space:]]*true' "connectors/$id/harness.toml"; then
  chat=yes
else
  chat=no
fi
placeholders() {
  grep -nHE '^[^#]*<[^>]+>' "$@"
}
if [ -d "$kit" ] || [ "$chat" = no ]; then
  step "files: the kit case for this connector"
  missing=''
  for f in "$kit/harness.toml" "$kit/stand-in.py" "$kit/expect.toml"; do
    [ -f "$f" ] || missing="$missing $f"
  done
  if [ -z "$missing" ]; then
    # Placeholders left from the templates, for example the stand-in's b"kit <HARNESS_ID> echo".
    left=$(placeholders "connectors/$id/harness.toml" "$kit/harness.toml" "$kit/expect.toml"
      awk '/^# ---- EDIT/ { e = 1 } /^# ---- end EDIT/ { e = 0 }
        e && /^[^#]*</ { print FILENAME ":" FNR ":" $0 }' "$kit/stand-in.py")
    if [ -z "$left" ]; then
      pass "files"
    else
      echo "template placeholders left:"
      echo "$left"
      fail "files (template placeholders left)"
    fi
  else
    echo "missing:$missing"
    echo "copy the templates in skills/adding-a-capture-harness/templates/ (see SKILL.md)"
    fail "files (missing:$missing)"
  fi
else
  step "files: a chat-mode connector (the kit runs it; no kit case of its own)"
  left=$(placeholders "connectors/$id/harness.toml")
  if [ -z "$left" ]; then
    pass "files (chat mode)"
  else
    echo "template placeholders left:"
    echo "$left"
    fail "files (template placeholders left)"
  fi
fi

# 1b. A Node harness needs NODE_USE_ENV_PROXY. Counted only when it fails.
program=$(sed -n 's/^[[:space:]]*program[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
  "connectors/$id/harness.toml" | head -n 1)
exe=''
[ -z "$program" ] || exe=$(command -v -- "$program" 2>/dev/null || true)
if [ -n "$exe" ] && [ -f "$exe" ]; then
  case $(head -c 128 "$exe" 2>/dev/null | head -n 1) in
    '#!'*node*)
      if grep -Eq '^[[:space:]]*NODE_USE_ENV_PROXY[[:space:]]*=[[:space:]]*"1"' \
        "connectors/$id/harness.toml"; then
        echo "node harness: $exe is a node script and NODE_USE_ENV_PROXY is set"
      else
        echo
        echo "################################################################################"
        echo "$exe is a node script, and connectors/$id/harness.toml does not set"
        echo "NODE_USE_ENV_PROXY = \"1\" in [capture.env]: Node ignores HTTPS_PROXY without it"
        echo "(D59), so a real run would record no model traffic"
        echo "################################################################################"
        fail "node proxy (NODE_USE_ENV_PROXY missing)"
      fi
      ;;
  esac
else
  echo "note: \`${program:-?}\` is not on PATH, so the node-proxy check did not run"
fi

# 2. Pins.
step "pins: sh scripts/pin-manifests.sh"
pins="crates/goalcup-protocol/pins/connector-dirs.txt"
if sh scripts/pin-manifests.sh; then
  # shellcheck disable=SC2086
  if ! diff=$(git diff HEAD -- $pins 2>/dev/null); then
    echo "git diff is unavailable here, so the pin diff was not checked"
    pass "pins (regenerated; diff not checked)"
  elif [ -z "$diff" ]; then
    echo "no pin change against HEAD"
    pass "pins"
  else
    echo "$diff"
    others=$(printf '%s\n' "$diff" | sed -n 's/^[+-]\([a-z][a-z0-9-]*\) [0-9a-f]\{64\}$/\1/p' |
      grep -v -x -F -- "$id" | sort -u | tr '\n' ' ')
    if [ -n "$others" ]; then
      echo "the pin diff also changes: $others"
      # shellcheck disable=SC2086
      changed=$(git diff --name-only HEAD -- $pins | tr '\n' ' ')
      echo "pin files were regenerated in place; \`git checkout -- ${changed% }\` undoes that,"
      echo "and also drops your own uncommitted pin line: re-run this script afterwards"
      fail "pins (other connectors changed: $others)"
    else
      echo "pin changes above are for '$id' only: commit them with the connector"
      pass "pins (uncommitted change for $id)"
    fi
  fi
else
  fail "pins (scripts/pin-manifests.sh failed)"
fi

# 3. Schema and pin tests.
step "schema: cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::"
if cargo test -p goalcup-protocol --test it -- harness_conformance:: connector_pins::; then
  pass "schema"
else
  fail "schema (field rules: docs/CONNECTOR-SCHEMA.md)"
fi

# 4. Goldens, without regeneration. The output is kept so the failing files can be named.
os=$(uname -s 2>/dev/null || echo unknown)
log=$(mktemp "${TMPDIR:-/tmp}/check-connector.XXXXXX")
trap 'rm -f "$log"' EXIT
for crate in goalcup-cli goalcup-daemon; do
  step "goldens: cargo test -p $crate --test it harness_contract_golden::"
  { cargo test -p "$crate" --test it harness_contract_golden:: 2>&1; echo "exit=$?"; } | tee "$log"
  if [ "$(tail -n 1 "$log")" = exit=0 ]; then
    pass "goldens ($crate)"
  else
    files=$(awk '/ differs from what the code now produces/ || / is missing \(/ {
      n = $1; sub(/.*\//, "", n); print n }' "$log" | sort -u | tr '\n' ' ')
    echo "golden files that differ: ${files:-(see the output above)}"
    echo "a file for '$id' ($id.json or $id.daemon.json): regenerate, then review the diff:"
    echo "  GOALCUP_REGEN_HARNESS_GOLDEN=1 cargo test -p $crate --test it harness_contract_golden::"
    echo "any other file did not change because of your connector: do not"
    echo "commit its regenerated form; restore it with git checkout and name it in the PR"
    fail "goldens ($crate): ${files:-see output}"
  fi
done
if [ "$os" != Darwin ]; then
  skipped=$((skipped + 1))
  echo
  echo "SKIPPED: the daemon's live golden ($id.daemon.json) is macOS-only ($os); a maintainer"
  echo "  checks it on a Mac"
  note "check-connector: SKIPPED daemon live golden (macOS-only; a maintainer checks it)"
fi

# 5. The harness kit, on macOS and Linux.
case $os in
  Darwin | Linux)
    step "kit: cargo build && cargo test -p goalcup-cli --test it harness_contract_kit::," \
      "then --test it connector_contract:: and --test full connector_contract::"
    if cargo build && cargo test -p goalcup-cli --test it harness_contract_kit:: &&
      cargo test -p goalcup-cli --test it connector_contract:: &&
      cargo test -p goalcup-cli --test full connector_contract::; then
      pass "kit"
    else
      fail "kit"
    fi
    ;;
  *)
    skipped=$((skipped + 1))
    echo
    echo "################################################################################"
    echo "SKIPPED: harness kit runs on macOS and Linux only; a maintainer runs it"
    echo "  ($os: harness_contract_kit compiles to zero tests here)"
    echo "################################################################################"
    note "check-connector: SKIPPED kit (macOS and Linux only; a maintainer runs it)"
    if [ "$allow_no_kit" = no ]; then
      failed=$((failed + 1))
      note "check-connector: FAIL kit skipped without --allow-no-kit"
    fi
    ;;
esac

printf '%s\n' "$summary"
echo "check-connector: $id: $passed passed, $failed failed, $skipped skipped"
[ "$failed" -eq 0 ] || exit 1
exit 0
