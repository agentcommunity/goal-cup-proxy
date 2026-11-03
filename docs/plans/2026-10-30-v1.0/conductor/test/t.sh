# Tiny test helpers. Source this file; call `finish` last.
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.invalid
_pass=0; _fail=0
ok() { _pass=$((_pass+1)); echo "ok $1"; }
fail() { _fail=$((_fail+1)); echo "fail $1 ${2:-}"; }
tmp() { mktemp -d; }
# fake_repo DIR: bare origin at DIR/origin.git (branch v1.0/integration) and a clone at DIR/repo
fake_repo() {
  local d=$1
  git init -q --bare "$d/origin.git"
  git clone -q "$d/origin.git" "$d/repo" 2>/dev/null
  git -C "$d/repo" checkout -q -b v1.0/integration
  echo seed > "$d/repo/seed.txt"
  git -C "$d/repo" add seed.txt
  git -C "$d/repo" commit -q -m seed
  git -C "$d/repo" push -q origin v1.0/integration
}
finish() { echo "$_pass passed, $_fail failed"; [ "$_fail" = 0 ]; }
