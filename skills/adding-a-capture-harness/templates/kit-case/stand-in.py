#!/usr/bin/python3
"""Kit stand-in template: one identity, one native file and one offline HTTPS echo.

Copy to crates/goalcup-cli/tests/fixtures/harness-kit/<HARNESS_ID>/stand-in.py and edit only
the EDIT block. Every value there must agree with this case's harness.toml and expect.toml.
The kit runs it as `python3 -I -S stand-in.py <port> <test-id> <marker> [<session-id>]`.
"""

import base64
import json
import os
import socket
import ssl
import sys
import time
from urllib.parse import unquote, urlsplit

assert sys.flags.isolated and sys.flags.no_site, "fixture Python must use -I -S"

# ---- EDIT ---------------------------------------------------------------------------------
BODY = b"kit <HARNESS_ID> echo"   # expect.toml wire_request and wire_response
CA_ENV = "NODE_EXTRA_CA_CERTS"    # the variable in harness.toml [capture.ca].env
IDENTITY = "none"                 # harness.toml identity source
SESSION_ID = "toy_case_42"        # stdout_json/stderr_line: expect.toml identity_id
STDERR_PREFIX = "session: "       # stderr_line: harness.toml identity prefix
MATCH_FIELD = "type"              # stdout_json: harness.toml match_field (dot path)
MATCH_VALUE = "session.started"   # stdout_json: harness.toml match_value
ID_FIELD = "session_id"           # stdout_json: harness.toml id_field (dot path)
# The native root as a directory: "HOME" for {run_home}, or the home.env name for
# {config_home}; then the relative part of the root after the token ("" for none).
NATIVE_BASE_ENV = "HOME"
NATIVE_DIR = "sessions"
NATIVE_NAME = "session-{id}.jsonl"  # the pattern, with {id} (no globs)
NATIVE_FORM = "append_only"         # or "snapshot"
FIRST = '{"message":"first"}\n'     # append_only: FIRST+SECOND is native_g1; snapshot: g1
SECOND = '{"message":"second"}\n'   # snapshot: native_g2
DECOY = '{"message":"decoy"}\n'     # written to the wrong-id file when the pattern has {id}
# ---- end EDIT -----------------------------------------------------------------------------

HOST = "api.e00.localhost"  # fixed: the kit's offline upstream
PATH = "/v1/echo"           # fixed: the path that echoes the body


def read_head(sock):
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
    connect = ("CONNECT %s:%s HTTP/1.1\r\nHost: %s:%s\r\n"
               "Proxy-Authorization: Basic %s\r\n\r\n") % (HOST, port, HOST, port, auth)
    sock.sendall(connect.encode())
    if b" 200 " not in read_head(sock).split(b"\r\n", 1)[0]:
        raise RuntimeError("CONNECT refused")
    context = ssl.create_default_context(cafile=os.environ[CA_ENV])
    tls = context.wrap_socket(sock, server_hostname=HOST)
    request = ("POST %s HTTP/1.1\r\nHost: %s:%s\r\n"
               "x-e00-test-id: %s\r\nContent-Length: %s\r\nConnection: close\r\n\r\n"
               % (PATH, HOST, port, test_id, len(BODY))).encode() + BODY
    tls.sendall(request)
    response = b""
    while True:
        part = tls.recv(4096)
        if not part:
            break
        response += part
    head, got = response.split(b"\r\n\r\n", 1)
    if b" 200 " not in head.split(b"\r\n", 1)[0] or got != BODY:
        raise RuntimeError("echo mismatch")


def nested(path, value):
    """{"a": {"b": value}} for the dot path "a.b"."""
    for part in reversed(path.split(".")):
        value = {part: value}
    return value


def merge(left, right):
    for key, value in right.items():
        if isinstance(value, dict) and isinstance(left.get(key), dict):
            merge(left[key], value)
        else:
            left[key] = value
    return left


def write_snapshot(path, text):
    pending = path + ".pending"
    with open(pending, "w") as stream:
        stream.write(text)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(pending, path)


port, test_id, marker = int(sys.argv[1]), sys.argv[2], sys.argv[3]
open(marker, "w").close()
if IDENTITY == "preassigned":
    session = sys.argv[4]
elif IDENTITY == "stdout_json":
    session = SESSION_ID
    event = merge(nested(MATCH_FIELD, MATCH_VALUE), nested(ID_FIELD, SESSION_ID))
    print(json.dumps(event), flush=True)
elif IDENTITY == "stderr_line":
    session = SESSION_ID
    print(STDERR_PREFIX + SESSION_ID, file=sys.stderr, flush=True)
else:
    session = None

root = os.path.join(os.environ[NATIVE_BASE_ENV], NATIVE_DIR)
os.makedirs(root, mode=0o700, exist_ok=True)
own = os.path.join(root, NATIVE_NAME.replace("{id}", session or ""))
os.makedirs(os.path.dirname(own), mode=0o700, exist_ok=True)
if NATIVE_FORM == "snapshot":
    write_snapshot(own, FIRST)
    time.sleep(0.75)
    write_snapshot(own, SECOND)
else:
    with open(own, "w") as stream:
        stream.write(FIRST)
        stream.flush()
        os.fsync(stream.fileno())
    time.sleep(0.75)
    with open(own, "a") as stream:
        stream.write(SECOND)
if session is not None and "{id}" in NATIVE_NAME:
    with open(os.path.join(root, NATIVE_NAME.replace("{id}", "wrong_id")), "w") as stream:
        stream.write(DECOY)
echo(port, test_id)
