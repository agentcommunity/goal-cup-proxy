#!/usr/bin/python3
"""Use a preassigned identity for one native file and one offline HTTPS echo."""

import base64
import os
import socket
import ssl
import sys
from urllib.parse import unquote, urlsplit

assert sys.flags.isolated and sys.flags.no_site, "fixture Python must use -I -S"

HOST = "api.e00.localhost"


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
    body = b"kit preassigned echo"
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


port, test_id, marker, identity = int(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
open(marker, "w").close()
native = os.path.join(os.environ["HOME"], "sessions")
os.makedirs(native, mode=0o700, exist_ok=True)
with open(os.path.join(native, "session-" + identity + ".jsonl"), "w") as stream:
    stream.write('{"message":"preassigned append"}\n')
with open(os.path.join(native, "session-wrong_preassigned.jsonl"), "w") as stream:
    stream.write('{"message":"preassigned decoy"}\n')
echo(port, test_id)
