#!/usr/bin/python3
"""Receive a child-only key, emit a stdout init identity, a snapshot and one offline echo.

The key is read from the environment and never written, printed or sent: the marker gets a
prefixed digest of it so the runner can tell which value arrived. The runner's key scan must
find nothing, while the launcher binds the identity from the same stdout it scans.
"""

import base64
import hashlib
import json
import os
import socket
import ssl
import sys
import time
from urllib.parse import unquote, urlsplit

assert sys.flags.isolated and sys.flags.no_site, "fixture Python must use -I -S"

HOST = "api.e00.localhost"
IDENTITY = "json_credential_7"
CREDENTIAL_ENV = "DEEPSEEK_API_KEY"


def head(sock):
    data = b""
    while b"\r\n\r\n" not in data:
        part = sock.recv(4096)
        if not part or len(data) > 16384:
            raise RuntimeError("incomplete HTTP head")
        data += part
    return data


def echo(port, test_id):
    proxy = urlsplit(os.environ["HTTPS_PROXY"])
    sock = socket.create_connection((proxy.hostname, proxy.port), timeout=10)
    user = unquote(proxy.username or "")
    password = unquote(proxy.password or "")
    auth = base64.b64encode((user + ":" + password).encode()).decode()
    request = ("CONNECT %s:%s HTTP/1.1\r\nHost: %s:%s\r\n"
               "Proxy-Authorization: Basic %s\r\n\r\n") % (HOST, port, HOST, port, auth)
    sock.sendall(request.encode())
    if b" 200 " not in head(sock).split(b"\r\n", 1)[0]:
        raise RuntimeError("CONNECT refused")
    tls = ssl.create_default_context(
        cafile=os.environ["NODE_EXTRA_CA_CERTS"]).wrap_socket(sock, server_hostname=HOST)
    body = b"kit credential-json echo"
    request = ("POST /v1/echo HTTP/1.1\r\nHost: %s:%s\r\n"
               "x-e00-test-id: %s\r\nContent-Length: %s\r\nConnection: close\r\n\r\n"
               % (HOST, port, test_id, len(body))).encode() + body
    tls.sendall(request)
    response = b""
    while True:
        part = tls.recv(4096)
        if not part:
            break
        response += part
    response_head, got = response.split(b"\r\n\r\n", 1)
    if b" 200 " not in response_head.split(b"\r\n", 1)[0] or got != body:
        raise RuntimeError("echo mismatch")


def wait_for_go(path, patience=60.0):
    """Wait for the runner to say it holds the first snapshot, so the rewrite is a second one.

    The runner creates `path` once the recorded chain holds the first snapshot. A fixed sleep
    here raced the recorder on a loaded host; this wait depends on the recorder, not the clock.
    """
    deadline = time.monotonic() + patience
    while not os.path.exists(path):
        if time.monotonic() > deadline:
            print("kit stand-in: the runner never released the rewrite", file=sys.stderr, flush=True)
            sys.exit(4)
        time.sleep(0.02)


def write_snapshot(path, message):
    pending = path + ".pending"
    with open(pending, "w") as stream:
        json.dump({"message": message}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(pending, path)


port, test_id, marker, go = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
key = os.environ.get(CREDENTIAL_ENV)
if not key:
    print("kit stand-in: no child credential", file=sys.stderr, flush=True)
    sys.exit(3)
with open(marker, "w") as stream:
    stream.write(hashlib.sha256(("kit-marker:" + key).encode()).hexdigest() + "\n")
del key
print(json.dumps({"type": "system", "subtype": "init", "session_id": IDENTITY}), flush=True)
home = os.environ["SYNTHCREDJSON_HOME"]
snapshot = os.path.join(home, "snapshots", IDENTITY + ".json")
write_snapshot(snapshot, "snapshot first")
decoy = os.path.join(home, "snapshots", "wrong_json_credential.json")
with open(decoy, "w") as stream:
    json.dump({"message": "credential decoy"}, stream)
wait_for_go(go)
write_snapshot(snapshot, "credential snapshot")
echo(port, test_id)
