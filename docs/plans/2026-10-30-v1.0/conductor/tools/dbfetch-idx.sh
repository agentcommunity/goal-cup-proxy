set -u
cd ~
python3 -I - <<'PY'
import os, urllib.parse
url=open(os.path.expanduser("~/.dburl")).read().strip()
u=urllib.parse.urlsplit(url); p={}
for k,v in (("host",u.hostname),("port",u.port),("user",u.username),("password",u.password),("dbname",u.path.lstrip("/"))):
    if v not in (None,""): p[k]=urllib.parse.unquote(str(v))
for k,v in urllib.parse.parse_qsl(u.query):
    if k in ("sslmode","sslrootcert","connect_timeout"): p[k]=v
p.setdefault("sslmode","require")
fd=os.open(os.path.expanduser("~/pg_service.conf"),os.O_WRONLY|os.O_CREAT|os.O_TRUNC,0o600)
os.write(fd,("[fetchrun]\n"+"".join(f"{k}={v}\n" for k,v in p.items())).encode()); os.close(fd)
PY
mkdir -p ~/idx
for id in $(cat ~/ids.txt); do
  PGSERVICEFILE=~/pg_service.conf PGSERVICE=fetchrun psql -X -q -At -v ON_ERROR_STOP=1 -v id="$id" -v staging=true -f ~/query.sql > ~/idx/$id.json 2>~/idx/$id.err
  echo "$id exit $? bytes $(wc -c < ~/idx/$id.json)"
done
rm -f ~/.dburl ~/pg_service.conf
tar cf /tmp/idx.tar -C ~ idx
