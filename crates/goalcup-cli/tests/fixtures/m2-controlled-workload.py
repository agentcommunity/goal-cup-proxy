#!/usr/bin/python3
"""NON-SHIPPING controlled workload of the M2 local production vertical. /usr/bin/python3 stdlib.

    m2-controlled-workload.py --version
    m2-controlled-workload.py CONFIG.json -- GOAL

The capture executable of the `m2-controlled-workload` development connector: `goalcup start`
launches it through the recording proxy (`HTTPS_PROXY`, the proxy CA in `NODE_EXTRA_CA_CERTS`)
with the goal as its one last argument. It reads the goal once, writes native JSONL under
`$HOME/native/`, exchanges traffic with the controlled endpoint (`api.e00.localhost`; the
authentication host `auth.e00.localhost` is a tunnel), writes `result.txt` in its working
directory and exits 0 when every exchange it made was byte-checked (1 otherwise, 64 on usage).
The helpers are the E02 load's (`spikes/m2-first-release/e02/early-load.py`), loaded by path.

CONFIG holds `mode`, `port`, `test_id`, `upstream_ca` and `out` (a directory outside the
recorder), and optionally `release` (a file whose appearance ends a stream, or with `hold` the
wait after the traffic) and `max_s` (the bound on either, default 60). Modes:
- plain          one echo, a 2 s SSE response and one WS exchange, eight native lines
- sparse         one SSE response of `events` 256-byte events `interval_ms` apart
- partial-native four native lines, then part of a fifth without its newline, then one echo
- split-secret   the E02 sentinel split across two request chunks, across a WS frame
                 boundary both ways, across two native writes 300 ms apart, and once in a
                 request cut off inside the secret
- fail-open      one echo (so the producer records a wire exchange), then a CONNECT to the
                 approved API host whose first bytes are not a TLS ClientHello: the proxy
                 tunnels it uncaptured (fail-open) and the workload reads until EOF
- stream         16 KiB echoes every 100 ms and 8 KiB WS messages every 200 ms on
                 connections kept open, one native line a second, until `release` or `max_s`
- load           the E02 Part B load (`schedule`), its report to `report`; `out/load-started.json`
                 marks its start
With `descendant` true the workload first forks one `sleep 300` that inherits its standard
output and error and outlives it (a held stdout); its pid goes to `out/descendant.pid`.
The pid goes to `out/pid` first, and `out/holding` marks a hold. Everything sent or received is
reported by size and sha256 to `out/events.jsonl` as it happens and to `out/report.json` at the
end: never a payload, the sentinel or the proxy credential.
"""

import sys

import importlib.util
import subprocess
import itertools
import json
import os
import socket
import threading
import time

sys.dont_write_bytecode = True

VERSION = "m2-controlled-workload 1"
HERE = os.path.dirname(os.path.abspath(__file__))
EARLY = os.path.join(HERE, "..", "..", "..", "..", "spikes", "m2-first-release", "e02",
                     "early-load.py")
spec = importlib.util.spec_from_file_location("early_load", EARLY)
E = importlib.util.module_from_spec(spec)
spec.loader.exec_module(E)


class Workload:
    def __init__(self, cfg, goal):
        self.cfg, self.out = cfg, cfg["out"]
        self.items, self.lock = [], threading.Lock()
        native_dir = os.path.join(os.environ["HOME"], "native")
        os.makedirs(native_dir, mode=0o700, exist_ok=True)
        self.native_path = os.path.join(native_dir, "e02-native.jsonl")
        os.environ["E02_PROXY_CA"] = os.environ["NODE_EXTRA_CA_CERTS"]
        url = "https://%s:%d"
        self.load = E.Load({"--endpoint": url % ("api.e00.localhost", cfg["port"]),
                            "--auth-endpoint": url % ("auth.e00.localhost", cfg["port"]),
                            "--upstream-ca": cfg["upstream_ca"],
                            "--native-path": self.native_path, "--test-id": cfg["test_id"]},
                           cfg.get("schedule", "smoke"))
        self.deadline = time.monotonic() + cfg.get("max_s", 60)
        self.goal = {"bytes": len(goal.encode()), "sha256": E.sha(goal.encode())}
        self.rng = E.Rng(0x723)

    def released(self):
        release = self.cfg.get("release")
        return bool(release and os.path.exists(release)) or time.monotonic() > self.deadline

    def note(self, **item):
        with self.lock:
            self.items.append(item)
            with open(os.path.join(self.out, "events.jsonl"), "a") as fh:
                fh.write(json.dumps(item) + "\n")

    def native(self, pieces, partial=b""):
        """Append each line; a (head, tail) pair is written in two halves 300 ms apart."""
        fd = os.open(self.native_path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            for piece in pieces:
                for i, part in enumerate(piece if isinstance(piece, tuple) else (piece,)):
                    time.sleep(0.3 if i else 0)
                    os.write(fd, part)
            os.write(fd, partial)
        finally:
            os.close(fd)

    def connect(self):
        return self.load.new_conn(self.load.api, self.load.proxy_ca)

    def echo(self, body, conn=None, chunks=None, sentinel=False):
        own, start = conn is None, time.time()
        try:
            cn, conn = self.connect() if own else (None, conn)
            status, resp = self.load.request(conn, "POST", "/v1/echo", body, close=own,
                                             chunks=chunks)
            self.note(lane="echo", path="/v1/echo", ok=status == 200 and resp == body,
                      sentinel=sentinel, start=start, end=time.time(), req_len=len(body),
                      req_sha256=E.sha(body), resp_len=len(resp), resp_sha256=E.sha(resp))
        except Exception as e:
            self.note(lane="echo", path="/v1/echo", ok=False, start=start, end=time.time(),
                      error=type(e).__name__)
            raise
        finally:
            if own and conn:
                conn.close()

    def sse(self, events, size, interval_ms):
        _cn, conn = self.connect()
        arrivals, start = [], time.time()
        target = "/v1/sse?events=%d&size=%d&interval_ms=%d" % (events, size, interval_ms)
        try:
            status, resp = self.load.request(conn, "POST", target, close=True,
                                             on_chunk=lambda _d: arrivals.append(time.time()))
        finally:
            conn.close()
        self.note(lane="sse", path="/v1/sse", ok=status == 200 and E.sse_ok(resp, events, size),
                  sentinel=False, start=start, end=time.time(), arrivals=arrivals,
                  resp_len=len(resp), resp_sha256=E.sha(resp))

    def ws(self, payloads, frag):
        cn, conn = self.load.ws_open()

        def control(op, data):
            if op == 0x9:
                conn.send(E.ws_frame(True, 0xA, data, self.rng.next().to_bytes(8, "big")[:4]))
            return "closed" if op == 0x8 else None

        try:
            for payload, sentinel in payloads:
                start = time.time()
                conn.send(E.ws_client_message(payload, frag, self.rng))
                op, echo, _frames = E.ws_read_message(conn.rf, control)
                self.note(lane="ws", path="/v1/ws", ok=op == 1 and echo == payload, conn=cn,
                          sentinel=sentinel, start=start, end=time.time(), req_len=len(payload),
                          req_sha256=E.sha(payload), resp_len=len(echo), resp_sha256=E.sha(echo))
        finally:
            self.load.ws_close(conn, self.rng)

    def cut_mid_secret(self):
        """A chunked request whose body stops inside a secret, then the connection closes."""
        body = E.with_sentinel(E.filler(7, 8192), 4001, 7)
        cut = body.index(E.SENTINEL_PREFIX) + 10
        _cn, conn = self.connect()
        conn.send(b"POST /v1/echo HTTP/1.1\r\nhost: api.e00.localhost:%d\r\nx-e00-test-id: %s\r\n"
                  b"transfer-encoding: chunked\r\n\r\n%x\r\n%s\r\n"
                  % (self.cfg["port"], self.cfg["test_id"].encode(), cut, body[:cut]))
        time.sleep(0.3)
        conn.close()
        self.note(lane="cut", path="/v1/echo", ok=True, sentinel=True, req_len=cut)

    def fail_open(self):
        """A CONNECT to the approved API host that then speaks something that is not TLS."""
        proxy, (host, port), start = self.load.proxy, self.load.api, time.time()
        sock = socket.create_connection((proxy.host, proxy.port), timeout=10)
        got = b""
        try:
            sock.sendall(("CONNECT %s:%d HTTP/1.1\r\nHost: %s:%d\r\nProxy-Authorization: Basic %s"
                          "\r\n\r\n" % (host, port, host, port, proxy._auth)).encode())
            head = b""
            while not head.endswith(b"\r\n\r\n"):
                chunk = sock.recv(1)
                if not chunk or len(head) > 8192:
                    raise EOFError("proxy closed during CONNECT")
                head += chunk
            if head.split(b"\r\n", 1)[0].split(b" ")[1:2] != [b"200"]:
                raise ConnectionError("CONNECT refused")
            sock.sendall(b"NOT-TLS\r\n\r\n")
            sock.settimeout(5)
            end = time.monotonic() + 5
            while time.monotonic() < end:
                try:
                    chunk = sock.recv(4096)
                except (socket.timeout, ConnectionError):
                    break
                if not chunk:
                    break
                got += chunk
        finally:
            sock.close()
        self.note(lane="fail_open", path="CONNECT", ok=True, start=start, end=time.time(),
                  resp_len=len(got))

    def stream(self):
        def echoes():
            conn = None
            for n in itertools.count():
                if self.released():
                    break
                try:
                    conn = conn or self.connect()[1]
                    self.echo(E.filler(n, 16384), conn=conn)
                except Exception:  # noqa: BLE001 - noted by echo; a fresh connection next
                    if conn:
                        conn.close()
                    conn = None
                time.sleep(0.1)
            if conn:
                conn.close()

        def messages():
            def payloads():
                for n in itertools.count():
                    if self.released():
                        return
                    time.sleep(0.2)
                    yield E.filler(1_000_000 + n, 8192), False
            while not self.released():
                try:
                    self.ws(payloads(), 4096)
                except Exception as e:  # noqa: BLE001
                    self.note(lane="ws", path="/v1/ws", ok=False, start=time.time(),
                              error=type(e).__name__)
                    time.sleep(0.2)

        def native():
            for s in itertools.count():
                if self.released():
                    break
                self.native([E.native_line(s, 0)])
                time.sleep(1)

        lanes = [threading.Thread(target=f) for f in (echoes, messages, native)]
        for lane in lanes:
            lane.start()
        for lane in lanes:
            lane.join()

    def run(self):
        mode = self.cfg["mode"]
        if self.cfg.get("descendant"):
            held = subprocess.Popen(["/bin/sleep", "300"], stdin=subprocess.DEVNULL)
            with open(os.path.join(self.out, "descendant.pid.tmp"), "w") as fh:
                fh.write("%d\n" % held.pid)
            os.replace(os.path.join(self.out, "descendant.pid.tmp"),
                       os.path.join(self.out, "descendant.pid"))
        if mode == "load":
            with open(os.path.join(self.out, "load-started.json"), "w") as fh:
                json.dump({"t0_wall": time.time()}, fh)
            report = self.load.run()
            with open(self.cfg["report"], "w", encoding="utf-8") as fh:
                json.dump(report, fh)
            return report["ok"]
        if mode == "plain":
            self.native([E.native_line(s, j) for s in range(2) for j in range(4)])
            self.echo(E.filler(1, 4096))
            self.sse(4, 232, 500)
            self.ws([(E.filler(2, 4096), False), (E.filler(3, 4096), False)], 1024)
        elif mode == "sparse":
            self.sse(self.cfg["events"], 232, self.cfg["interval_ms"])
        elif mode == "partial-native":
            self.native([E.native_line(s, 0) for s in range(4)], E.native_line(4, 0)[:500])
            self.echo(E.filler(4, 2048))
        elif mode == "split-secret":
            line = E.native_line(45, 0, sentinel_index=45)
            cut = line.index(E.SENTINEL_PREFIX) + 10
            self.native([E.native_line(44, 0), (line[:cut], line[cut:])])
            body = E.with_sentinel(E.filler(5, 16384), 8001, 5)
            self.echo(body, chunks=[body[:8011], body[8011:]], sentinel=True)
            self.ws([(E.with_sentinel(E.filler(6, 16384), 4096 - 20, 6), True)], 4096)
            self.cut_mid_secret()
        elif mode == "fail-open":
            self.echo(E.filler(7, 2048))
            self.fail_open()
        elif mode == "stream":
            self.stream()
        else:
            return None
        if self.cfg.get("hold"):
            open(os.path.join(self.out, "holding"), "w").close()
            while not self.released():
                time.sleep(0.05)
        return all(item["ok"] for item in self.items)


def main(argv):
    if argv == ["--version"]:
        print(VERSION)
        return 0
    if len(argv) != 3 or argv[1] != "--":
        sys.stderr.write(__doc__)
        return 64
    with open(argv[0], encoding="utf-8") as fh:
        cfg = json.load(fh)
    work = Workload(cfg, argv[2])
    with open(os.path.join(work.out, "pid.tmp"), "w") as fh:
        fh.write("%d\n" % os.getpid())
    os.replace(os.path.join(work.out, "pid.tmp"), os.path.join(work.out, "pid"))
    ok = work.run()
    if ok is None:
        sys.stderr.write(__doc__)
        return 64
    with open("result.txt", "w", encoding="utf-8") as fh:
        fh.write("m2-controlled-workload %s\n" % ("ok" if ok else "failed"))
    native = b""
    if os.path.exists(work.native_path):
        with open(work.native_path, "rb") as fh:
            native = fh.read()
    with open(os.path.join(work.out, "report.json"), "w", encoding="utf-8") as fh:
        json.dump({"version": VERSION, "mode": cfg["mode"], "ok": ok, "goal": work.goal,
                   "items": work.items, "native": {"bytes": len(native), "sha256": E.sha(native)}},
                  fh)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
