#!/usr/bin/python3
"""One offline native write, then a second once the runner says the first is recorded, and one
HTTPS echo through the recording proxy."""

import base64
import os
import socket
import ssl
import sys
import time
from urllib.parse import unquote, urlsplit

assert sys.flags.isolated and sys.flags.no_site, "fixture Python must use -I -S"

HOST = "api.e00.localhost"


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
    context = ssl.create_default_context(cafile=os.environ["NODE_EXTRA_CA_CERTS"])
    tls = context.wrap_socket(sock, server_hostname=HOST)
    body = b"kit controlled echo"
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
    head, got = response.split(b"\r\n\r\n", 1)
    if b" 200 " not in head.split(b"\r\n", 1)[0] or got != body:
        raise RuntimeError("echo mismatch")


def wait_for_go(path, patience=60.0):
    """Wait for the runner to say it holds the first line, so the append is a later write.

    The runner creates `path` once the recorded chain holds the first line. A fixed sleep here
    raced the recorder on a loaded host; this wait depends on the recorder, not the clock.
    """
    deadline = time.monotonic() + patience
    while not os.path.exists(path):
        if time.monotonic() > deadline:
            print("kit stand-in: the runner never released the append", file=sys.stderr, flush=True)
            sys.exit(4)
        time.sleep(0.02)


port, test_id, marker, go = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
open(marker, "w").close()
native = os.path.join(os.environ["HOME"], "native")
os.makedirs(native, mode=0o700)
path = os.path.join(native, "controlled.jsonl")
with open(path, "w") as stream:
    stream.write('{"message":"controlled first"}\n')
    stream.flush()
    os.fsync(stream.fileno())
wait_for_go(go)
with open(path, "a") as stream:
    stream.write('{"message":"controlled native"}\n')
echo(port, test_id)
