command -v psql >/dev/null || (dnf install -y -q postgresql16 >/dev/null 2>&1 || dnf install -y -q postgresql15 >/dev/null 2>&1 || dnf install -y -q postgresql >/dev/null 2>&1)
command -v psql && psql --version
