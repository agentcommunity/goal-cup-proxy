#!/usr/bin/python3
"""Receive a child-only key, emit a deferred token identity, append native bytes and echo once.

The key is read from the environment and never written, printed or sent: the marker gets a
prefixed digest of it so the runner can tell which value arrived, and the request carries no
credential. The runner's key scan must find nothing.
"""

import base64
import hashlib
import os
import socket
import ssl
import sys
from urllib.parse import unquote, urlsplit

assert sys.flags.isolated and sys.flags.no_site, "fixture Python must use -I -S"

HOST = "api.e00.localhost"
IDENTITY = "credential_token_7"
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
    body = b"kit credential echo"
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


port, test_id, marker = int(sys.argv[1]), sys.argv[2], sys.argv[3]
key = os.environ.get(CREDENTIAL_ENV)
if not key:
    print("kit stand-in: no child credential", file=sys.stderr, flush=True)
    sys.exit(3)
with open(marker, "w") as stream:
    stream.write(hashlib.sha256(("kit-marker:" + key).encode()).hexdigest() + "\n")
del key
print("scope: " + IDENTITY, file=sys.stderr, flush=True)
native = os.path.join(os.environ["HOME"], "sessions")
os.makedirs(native, mode=0o700, exist_ok=True)
with open(os.path.join(native, "session-" + IDENTITY + ".jsonl"), "w") as stream:
    stream.write('{"message":"credential append"}\n')
with open(os.path.join(native, "session-wrong_credential_token.jsonl"), "w") as stream:
    stream.write('{"message":"credential decoy"}\n')
echo(port, test_id)
