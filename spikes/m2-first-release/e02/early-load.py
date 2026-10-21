#!/usr/bin/python3
"""NON-SHIPPING E02 Part B deterministic recorder load (Goal Cup M2). /usr/bin/python3 stdlib only.

    early-load.py plan [--schedule full|smoke]      print the exact byte schedule and totals (JSON)
    early-load.py selfcheck                         offline checks of this script (exit 0 = ok)
    early-load.py run --schedule full|smoke --endpoint URL --auth-endpoint URL
                      --upstream-ca PATH --native-path PATH --test-id ID --report PATH

`run` is the workload of the E00/E01 runner with `--controlled-endpoint`: every connection goes
through the runner's proxy (`HTTPS_PROXY`, credential in the URL, held in memory only) with the
per-launch proxy CA from `E02_PROXY_CA`; the auth host is a tunnel to the endpoint (its CA is
`--upstream-ca`). It uses only E00's public fixture routes: `POST /v1/echo`,
`POST /v1/sse?events&size&interval_ms`, `GET /v1/ws?frag` and `POST /v1/bulk?size` (the stall
trigger), plus the auth host.

Schedule (`full`): six 300 s phases, dense/sparse alternating (D S D S D S), 1800 s in all.
- Dense, per 1 s tick, 128 KiB of selected decoded payload counted in both directions:
  HTTP/SSE half: one 16 KiB echo (16 KiB up + 16 KiB down) and one SSE response of 8 events
  x 4088 B plus one 64-byte tag event (32 KiB down); WS half: two 16 KiB client text messages
  (4 KiB frames, a ping after the first frame) echoed by the endpoint in 4 KiB frames (64 KiB).
- Sparse: sixteen interleaved SSE streams (offsets 0.625 s apart), each receiving one 240-byte
  event every 10 s: 3 requests x 10 events per phase and stream, and after the last event of a
  request one tag event, so a request is still 10 x 256 B.
- Unique content (F-16): every echo body and WS message starts with its own tag (lane, tick,
  message). Every SSE request carries a unique tag body (56 B dense, 152 B sparse; outside the
  counted payload) and asks the endpoint (`x-e00-split-at`) to send it back as the response's
  last event, split across two chunks. So every request, response and WS message has its own
  sha256, and the cross-check (`early-crosscheck.py`) can tell which one is missing.
- Native: 4 KiB/s for the whole run, four 1024-byte JSONL lines per second.
- Exact logical payload: 117 964 800 + 368 640 + 7 372 800 = 125 706 240 B = 119.8828125 MiB
  (the tags are inside it: the SSE event sizes shrank to make room).
- Stall trigger at 720 s (minute 12, inside the second dense phase): one empty
  `POST /v1/bulk?size=0` on its own connection (0 payload bytes).
- Renewal: echo connections every 10 requests, SSE every 15, WS sessions every 30 ticks, and
  every lane connection at the end of each phase. Overlap: all lanes run concurrently.
- Secret sentinel (a synthetic `sk-ant-` value, never written to the report): in the echo
  request body of dense ticks 30, 90, ... (split across two request chunks 50 ms apart), in the
  first WS message of the same ticks (split across the 4096-byte frame boundary, both ways),
  and in one native line per minute (written in two halves 300 ms apart). Sizes never change.

`smoke` is the same with 30 s phases (180 s in all) and the stall at 72 s. It is a plumbing
check, never a measurement.

The report holds sizes and sha256 digests of everything sent and received, per-tick
lateness, and totals; never a payload, the sentinel or the proxy credential. `run` exits 0
when every exchange was byte-checked and the totals equal the plan, 1 when a check failed, 3
when the load could not run, 64 on usage.
"""

import base64
import hashlib
import json
import os
import re
import socket
import ssl
import sys
import threading
import time
from urllib.parse import urlsplit, unquote

MASK = (1 << 64) - 1
SCHEDULES = {
    "full": {"phase_s": 300, "phases": "DSDSDS", "stall_at_s": 720},
    "smoke": {"phase_s": 30, "phases": "DSDSDS", "stall_at_s": 72},
}
ECHO_BYTES = 16384
SSE_EVENTS, SSE_SIZE, SSE_INTERVAL_MS = 8, 4064, 100  # 8 x (24 + 4064) + (6 + 56 + 2) = 32768
SSE_TAG_BYTES = 56  # the dense SSE request body, echoed back as the last event
WS_MESSAGES, WS_BYTES, WS_FRAG = 2, 16384, 4096
SPARSE_STREAMS, SPARSE_SIZE, SPARSE_EVERY_S = 16, 216, 10  # 24 + 216 = 240 B per event
SPARSE_SHARE = 256  # bytes per sparse event once the request's tag event is shared out
NATIVE_LINES, NATIVE_LINE_BYTES = 4, 1024
RENEW_ECHO, RENEW_SSE, RENEW_WS = 10, 15, 30
SENTINEL_EVERY, SENTINEL_AT = 60, 30  # dense ticks 30, 90, 150, ...
NATIVE_SENTINEL_AT = 45  # seconds 45, 105, ... of the run
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MAX_ERRORS = 100


class Rng:
    def __init__(self, seed):
        self.s = seed & MASK

    def next(self):
        self.s = (self.s + 0x9E3779B97F4A7C15) & MASK
        z = self.s
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)


def make_pool(n=1 << 20, seed=0xE02B):
    """Lower-case words of 2-9 letters separated by single spaces: nothing in it is a secret
    shape, so the only thing the redactor should replace is the sentinel."""
    rng, out, size = Rng(seed), [], 0
    while size < n:
        w = "".join(chr(97 + rng.next() % 26) for _ in range(2 + rng.next() % 8))
        out.append(w)
        size += len(w) + 1
    return (" ".join(out))[:n].encode("ascii")


POOL = make_pool()


def filler(index, n):
    off = (index * 7919) % (len(POOL) - n)
    return POOL[off:off + n]


def sentinel(i):
    """A synthetic value in the shape of the T04 `anthropic_key` pattern; built at run time so
    the literal never appears in source or report."""
    return ("sk-" + "ant-" + "api03-" + "E02" + "sentinel" + "%06d" % i + "x" * 30).encode("ascii")


SENTINEL_PREFIX = ("sk-" + "ant-" + "api03-" + "E02" + "sentinel").encode("ascii")


def with_sentinel(body, at, i):
    """`body` with ` <sentinel> ` placed so the sentinel starts at byte `at`; same length."""
    s = sentinel(i)
    return body[:at - 1] + b" " + s + b" " + body[at + len(s) + 1:]


def tagged(body, tag):
    """`body` with its first bytes replaced by `tag`; same length."""
    if len(tag) > len(body):
        raise ValueError("tag longer than body")
    return tag + body[len(tag):]


def tag_event_len(n):
    """The endpoint's echo of an `n`-byte request body: `data: <body>\n\n`."""
    return len(b"data: ") + n + len(b"\n\n")


def sparse_tag_bytes(events):
    """The sparse request tag that brings a request of `events` events to `events` x 256 B."""
    return events * (SPARSE_SHARE - sse_event_len(0, SPARSE_SIZE)) - tag_event_len(0)


def echo_body(k):
    return tagged(filler(10_000_000 + k, ECHO_BYTES), b"e02 echo %07d " % k)


def ws_payload(k, m):
    return tagged(filler(20_000_000 + k * WS_MESSAGES + m, WS_BYTES), b"e02 ws %07d %d " % (k, m))


def sse_tag(k):
    return tagged(filler(40_000_000 + k, SSE_TAG_BYTES), b"e02 sse %07d " % k)


def sparse_tag(j, ph, r, events):
    return tagged(filler(50_000_000 + (ph * SPARSE_STREAMS + j) * 100 + r, sparse_tag_bytes(events)),
                  b"e02 sparse %02d %d %d " % (j, ph, r))


def plan(name):
    sch = SCHEDULES[name]
    p, phases = sch["phase_s"], sch["phases"]
    dense_ticks = p * phases.count("D")
    sparse_phases = phases.count("S")
    events_per_req = min(10, p // SPARSE_EVERY_S)
    reqs_per_phase = (p // SPARSE_EVERY_S) // events_per_req
    sparse_event = sse_event_len(0, SPARSE_SIZE)
    sparse_request = events_per_req * sparse_event + tag_event_len(sparse_tag_bytes(events_per_req))
    dense_sse = SSE_EVENTS * sse_event_len(0, SSE_SIZE) + tag_event_len(SSE_TAG_BYTES)
    dense_tick = 2 * ECHO_BYTES + dense_sse + 2 * WS_MESSAGES * WS_BYTES
    duration = p * len(phases)
    out = {
        "schedule": name, "phase_s": p, "phases": phases, "duration_s": duration,
        "stall_at_s": sch["stall_at_s"],
        "dense": {"ticks": dense_ticks, "bytes_per_tick": dense_tick,
                  "echo_up": ECHO_BYTES, "echo_down": ECHO_BYTES, "sse_down": dense_sse,
                  "sse_events": SSE_EVENTS, "sse_size_param": SSE_SIZE,
                  "sse_tag_bytes": SSE_TAG_BYTES, "sse_tag_event_bytes": tag_event_len(SSE_TAG_BYTES),
                  "ws_messages_each_way": WS_MESSAGES, "ws_message_bytes": WS_BYTES,
                  "ws_frag": WS_FRAG, "bytes": dense_ticks * dense_tick},
        "sparse": {"streams": SPARSE_STREAMS, "phases": sparse_phases,
                   "requests_per_phase": reqs_per_phase, "events_per_request": events_per_req,
                   "event_bytes": sparse_event, "event_every_s": SPARSE_EVERY_S,
                   "tag_bytes": sparse_tag_bytes(events_per_req),
                   "tag_event_bytes": tag_event_len(sparse_tag_bytes(events_per_req)),
                   "request_bytes": sparse_request,
                   "bytes": sparse_phases * SPARSE_STREAMS * reqs_per_phase * sparse_request},
        "native": {"lines_per_s": NATIVE_LINES, "line_bytes": NATIVE_LINE_BYTES,
                   "bytes": duration * NATIVE_LINES * NATIVE_LINE_BYTES},
        "sentinels": {"echo": len(sentinel_ticks(dense_ticks)), "ws": len(sentinel_ticks(dense_ticks)),
                      "native": len([s for s in range(duration)
                                     if s % SENTINEL_EVERY == NATIVE_SENTINEL_AT])},
        "counts": {"echo_exchanges": dense_ticks,
                   "sse_exchanges": dense_ticks + sparse_phases * SPARSE_STREAMS * reqs_per_phase,
                   "ws_client_messages": dense_ticks * WS_MESSAGES,
                   "ws_server_messages": dense_ticks * WS_MESSAGES,
                   "bulk_exchanges": 1, "auth_requests": 1,
                   # Events the endpoint sends, each request's tag event included.
                   "sse_events": dense_ticks * (SSE_EVENTS + 1)
                   + sparse_phases * SPARSE_STREAMS * reqs_per_phase * (events_per_req + 1)},
        # The SSE request tags go up and are not counted in the payload above.
        "sse_tag_up_bytes": dense_ticks * SSE_TAG_BYTES
        + sparse_phases * SPARSE_STREAMS * reqs_per_phase * sparse_tag_bytes(events_per_req),
    }
    out["total_bytes"] = out["dense"]["bytes"] + out["sparse"]["bytes"] + out["native"]["bytes"]
    out["total_mib"] = out["total_bytes"] / 1048576
    return out


def sentinel_ticks(dense_ticks):
    return [t for t in range(dense_ticks) if t % SENTINEL_EVERY == SENTINEL_AT]


def sse_event_len(i, size):
    """The endpoint's `data: {"i":<i>,"pad":"<size bytes>"}\\n\\n`."""
    return len(('data: {"i":%d,"pad":"' % i).encode()) + size + len(b'"}\n\n')


# ---------------------------------------------------------------- WebSocket framing


def ws_frame(fin, opcode, payload, mask_key=None):
    b0 = (0x80 if fin else 0) | opcode
    n = len(payload)
    m = 0x80 if mask_key is not None else 0
    if n < 126:
        head = bytes([b0, m | n])
    elif n < 65536:
        head = bytes([b0, m | 126]) + n.to_bytes(2, "big")
    else:
        head = bytes([b0, m | 127]) + n.to_bytes(8, "big")
    if mask_key is None:
        return head + payload
    return head + mask_key + bytes(b ^ mask_key[i % 4] for i, b in enumerate(payload))


def ws_client_message(payload, frag, mask_rng, ping_after_first=True):
    """Client frames of one text message, `frag` bytes each, a ping after the first."""
    frames = []
    pieces = [payload[i:i + frag] for i in range(0, len(payload), frag)] or [b""]
    for i, piece in enumerate(pieces):
        op = 0x1 if i == 0 else 0x0
        frames.append(ws_frame(i == len(pieces) - 1, op, piece,
                               mask_rng.next().to_bytes(8, "big")[:4]))
        if i == 0 and ping_after_first and len(pieces) > 1:
            frames.append(ws_frame(True, 0x9, b"e02", mask_rng.next().to_bytes(8, "big")[:4]))
    return b"".join(frames)


def read_exact(rf, n):
    data = rf.read(n)
    if data is None or len(data) != n:
        raise EOFError("connection closed mid-frame")
    return data


def ws_read_frame(rf):
    b0, b1 = read_exact(rf, 2)
    n = b1 & 0x7F
    if n == 126:
        n = int.from_bytes(read_exact(rf, 2), "big")
    elif n == 127:
        n = int.from_bytes(read_exact(rf, 8), "big")
    key = read_exact(rf, 4) if b1 & 0x80 else None
    data = read_exact(rf, n)
    if key:
        data = bytes(b ^ key[i % 4] for i, b in enumerate(data))
    return bool(b0 & 0x80), b0 & 0x0F, data


def ws_read_message(rf, on_control):
    """Reassemble one data message; control frames go to `on_control(opcode, payload)`."""
    parts, opcode, frames = [], None, 0
    while True:
        fin, op, data = ws_read_frame(rf)
        if op >= 0x8:
            if on_control(op, data) == "closed":
                raise EOFError("closed by peer")
            continue
        frames += 1
        if opcode is None:
            if op == 0:
                raise ValueError("stray continuation")
            opcode = op
        elif op != 0:
            raise ValueError("interleaved data message")
        parts.append(data)
        if fin:
            return opcode, b"".join(parts), frames


# ---------------------------------------------------------------- HTTP over the proxy


class Proxy:
    def __init__(self, url):
        u = urlsplit(url)
        if u.scheme != "http" or not u.hostname or not u.port:
            raise ValueError("HTTPS_PROXY must be http://user:pass@host:port")
        self.host, self.port = u.hostname, u.port
        cred = "%s:%s" % (unquote(u.username or ""), unquote(u.password or ""))
        self._auth = base64.b64encode(cred.encode()).decode()

    def __repr__(self):  # never the credential
        return "Proxy(%s:%d)" % (self.host, self.port)

    def connect(self, host, port, cafile):
        s = socket.create_connection((self.host, self.port), timeout=60)
        try:
            s.sendall(("CONNECT %s:%d HTTP/1.1\r\nHost: %s:%d\r\nProxy-Authorization: Basic %s"
                       "\r\n\r\n" % (host, port, host, port, self._auth)).encode())
            head = b""
            while not head.endswith(b"\r\n\r\n"):
                c = s.recv(1)
                if not c or len(head) > 8192:
                    raise EOFError("proxy closed during CONNECT")
                head += c
            status = head.split(b"\r\n", 1)[0].split(b" ")
            if len(status) < 2 or status[1] != b"200":
                raise ConnectionError("CONNECT refused: %s" % status[1:2])
            ctx = ssl.create_default_context(cafile=cafile)
            return Conn(ctx.wrap_socket(s, server_hostname=host))
        except BaseException:
            s.close()
            raise


class Conn:
    def __init__(self, sock):
        self.sock, self.rf = sock, sock.makefile("rb")

    def send(self, data):
        self.sock.sendall(data)

    def head(self):
        line = self.rf.readline(65536)
        if not line:
            raise EOFError("connection closed before the response")
        status = int(line.split(b" ")[1])
        headers = {}
        while True:
            line = self.rf.readline(65536)
            if line in (b"\r\n", b"\n", b""):
                break
            k, _, v = line.decode("latin-1").partition(":")
            headers[k.strip().lower()] = v.strip()
        return status, headers

    def body(self, headers, on_chunk=None):
        if headers.get("transfer-encoding", "").lower() == "chunked":
            out = []
            while True:
                size = int(self.rf.readline(1024).split(b";")[0].strip(), 16)
                if size == 0:
                    self.rf.readline(1024)
                    return b"".join(out)
                data = read_exact(self.rf, size)
                if on_chunk:
                    on_chunk(data)
                out.append(data)
                self.rf.readline(1024)
        return read_exact(self.rf, int(headers.get("content-length", "0")))

    def close(self):
        for f in (self.rf.close, self.sock.close):
            try:
                f()
            except OSError:
                pass


def sha(data):
    return hashlib.sha256(data).hexdigest()


# ---------------------------------------------------------------- the run


class Load:
    def __init__(self, args, schedule):
        self.plan = plan(schedule)
        self.proxy = Proxy(os.environ["HTTPS_PROXY"])
        self.proxy_ca = os.environ["E02_PROXY_CA"]
        api = urlsplit(args["--endpoint"])
        auth = urlsplit(args["--auth-endpoint"])
        self.api = (api.hostname, api.port)
        self.auth = (auth.hostname, auth.port)
        self.upstream_ca = args["--upstream-ca"]
        self.native_path = args["--native-path"]
        self.test_id = args["--test-id"]
        self.lock = threading.Lock()
        self.exchanges, self.ws_messages, self.errors, self.lateness = [], [], [], {}
        self.native = {}
        self.stall = {}
        self.conn_seq = 0
        self.t0 = None
        self.t0_wall = None

    def now(self):
        return time.monotonic() - self.t0

    def wait_until(self, t):
        d = t - self.now()
        if d > 0:
            time.sleep(d)

    def error(self, where, e):
        with self.lock:
            if len(self.errors) < MAX_ERRORS:
                self.errors.append({"where": where, "t_s": round(self.now(), 3),
                                    "error": "%s: %s" % (type(e).__name__, str(e)[:200])})

    def new_conn(self, target, ca):
        with self.lock:
            self.conn_seq += 1
            n = self.conn_seq
        return n, self.proxy.connect(target[0], target[1], ca)

    def request(self, conn, method, target, body=b"", close=False, chunks=None, on_chunk=None,
                host=None, headers=()):
        host = host or self.api
        head = ["%s %s HTTP/1.1" % (method, target), "host: %s:%d" % host,
                "x-e00-test-id: %s" % self.test_id, "content-type: text/plain"]
        head += ["%s: %s" % kv for kv in headers]
        if chunks is not None:
            head.append("transfer-encoding: chunked")
        else:
            head.append("content-length: %d" % len(body))
        if close:
            head.append("connection: close")
        conn.send(("\r\n".join(head) + "\r\n\r\n").encode())
        if chunks is not None:
            for i, piece in enumerate(chunks):
                if i:
                    time.sleep(0.05)
                conn.send(b"%x\r\n" % len(piece) + piece + b"\r\n")
            conn.send(b"0\r\n\r\n")
        elif body:
            conn.send(body)
        status, headers = conn.head()
        return status, conn.body(headers, on_chunk)

    def record(self, **kw):
        with self.lock:
            self.exchanges.append(kw)

    # -- lanes

    def dense_ticks(self):
        """(global dense tick index, scheduled time, phase index, last tick of phase)."""
        p, out, k = self.plan["phase_s"], [], 0
        for ph, kind in enumerate(self.plan["phases"]):
            if kind != "D":
                continue
            for i in range(p):
                out.append((k, ph * p + i, ph, i == p - 1))
                k += 1
        return out

    def lane_echo(self):
        conn, cn, used, late = None, None, 0, []
        for k, t, _ph, last in self.dense_ticks():
            self.wait_until(t)
            start = self.now()
            late.append(round(start - t, 3))
            body = echo_body(k)
            sent_chunks = None
            s_on = k % SENTINEL_EVERY == SENTINEL_AT
            if s_on:
                at = 8001
                body = with_sentinel(body, at, k)
                sent_chunks = [body[:at + 10], body[at + 10:]]
            try:
                if conn is None:
                    cn, conn = self.new_conn(self.api, self.proxy_ca)
                    used = 0
                used += 1
                close = used == RENEW_ECHO or last
                status, resp = self.request(conn, "POST", "/v1/echo", body, close=close,
                                            chunks=sent_chunks)
                ok = status == 200 and resp == body
                self.record(lane="echo", path="/v1/echo", tick=k, conn=cn, sched_s=t,
                            start_s=round(start, 3), end_s=round(self.now(), 3), ok=ok,
                            sentinel=s_on, req_len=len(body), req_sha256=sha(body),
                            resp_len=len(resp), resp_sha256=sha(resp))
                if close:
                    conn.close()
                    conn = None
            except Exception as e:  # noqa: BLE001 - recorded, then a fresh connection
                self.error("echo tick %d" % k, e)
                self.record(lane="echo", path="/v1/echo", tick=k, conn=cn, sched_s=t,
                            start_s=round(start, 3), end_s=round(self.now(), 3), ok=False,
                            sentinel=s_on, req_len=len(body), req_sha256=sha(body),
                            resp_len=None, resp_sha256=None)
                if conn:
                    conn.close()
                conn = None
        self.lateness["echo"] = late

    def lane_sse(self):
        conn, cn, used, late = None, None, 0, []
        want = self.plan["dense"]["sse_down"]
        target = "/v1/sse?events=%d&size=%d&interval_ms=%d" % (SSE_EVENTS, SSE_SIZE, SSE_INTERVAL_MS)
        for k, t, _ph, last in self.dense_ticks():
            self.wait_until(t)
            start = self.now()
            late.append(round(start - t, 3))
            body = sse_tag(k)
            try:
                if conn is None:
                    cn, conn = self.new_conn(self.api, self.proxy_ca)
                    used = 0
                used += 1
                close = used == RENEW_SSE or last
                status, resp = self.request(conn, "POST", target, body, close=close,
                                            headers=[("x-e00-split-at", str(len(body) // 2))])
                ok = status == 200 and len(resp) == want and sse_ok(resp, SSE_EVENTS, SSE_SIZE,
                                                                    tail=body)
                self.record(lane="sse", path="/v1/sse", tick=k, conn=cn, sched_s=t,
                            start_s=round(start, 3), end_s=round(self.now(), 3), ok=ok,
                            sentinel=False, req_len=len(body), req_sha256=sha(body),
                            resp_len=len(resp), resp_sha256=sha(resp), sse_events=SSE_EVENTS + 1)
                if close:
                    conn.close()
                    conn = None
            except Exception as e:  # noqa: BLE001
                self.error("sse tick %d" % k, e)
                self.record(lane="sse", path="/v1/sse", tick=k, conn=cn, sched_s=t,
                            start_s=round(start, 3), end_s=round(self.now(), 3), ok=False,
                            sentinel=False, req_len=len(body), req_sha256=sha(body),
                            resp_len=None, resp_sha256=None, sse_events=None)
                if conn:
                    conn.close()
                conn = None
        self.lateness["sse"] = late

    def ws_open(self):
        cn, conn = self.new_conn(self.api, self.proxy_ca)
        key = base64.b64encode(hashlib.sha256(b"e02-%d" % cn).digest()[:16]).decode()
        conn.send(("GET /v1/ws?frag=%d HTTP/1.1\r\nhost: %s:%d\r\nx-e00-test-id: %s\r\n"
                   "upgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-key: %s\r\n"
                   "sec-websocket-version: 13\r\n\r\n"
                   % (WS_FRAG, self.api[0], self.api[1], self.test_id, key)).encode())
        status, headers = conn.head()
        accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        if status != 101 or headers.get("sec-websocket-accept") != accept:
            conn.close()
            raise ConnectionError("upgrade refused: %d" % status)
        return cn, conn

    def ws_close(self, conn, rng):
        try:
            conn.send(ws_frame(True, 0x8, (1000).to_bytes(2, "big"), rng.next().to_bytes(8, "big")[:4]))
            while True:
                _fin, op, _data = ws_read_frame(conn.rf)
                if op == 0x8:
                    break
        except (OSError, EOFError, ValueError):
            pass
        conn.close()

    def lane_ws(self):
        rng = Rng(0xE02F5)
        conn, cn, used, late, session = None, None, 0, [], 0

        def on_control(op, data):
            if op == 0x9:
                conn.send(ws_frame(True, 0xA, data, rng.next().to_bytes(8, "big")[:4]))
            elif op == 0x8:
                return "closed"
            return None

        for k, t, _ph, last in self.dense_ticks():
            self.wait_until(t)
            start = self.now()
            late.append(round(start - t, 3))
            try:
                if conn is None:
                    cn, conn = self.ws_open()
                    session += 1
                    used = 0
                used += 1
                for m in range(WS_MESSAGES):
                    payload = ws_payload(k, m)
                    s_on = m == 0 and k % SENTINEL_EVERY == SENTINEL_AT
                    if s_on:
                        payload = with_sentinel(payload, WS_FRAG - 20, k)
                    t_send = self.now()
                    conn.send(ws_client_message(payload, WS_FRAG, rng))
                    op, echo, frames = ws_read_message(conn.rf, on_control)
                    ok = op == 0x1 and echo == payload
                    with self.lock:
                        for d, data in (("client", payload), ("server", echo)):
                            self.ws_messages.append({
                                "tick": k, "m": m, "session": session, "conn": cn, "dir": d,
                                "len": len(data), "sha256": sha(data), "sentinel": s_on,
                                "ok": ok, "t_s": round(t_send, 3),
                                "end_s": round(self.now(), 3), "server_frames": frames})
                if used == RENEW_WS or last:
                    self.ws_close(conn, rng)
                    conn = None
            except Exception as e:  # noqa: BLE001
                self.error("ws tick %d" % k, e)
                with self.lock:
                    self.ws_messages.append({"tick": k, "m": None, "session": session,
                                             "conn": cn, "dir": "error", "ok": False,
                                             "t_s": round(start, 3)})
                if conn:
                    conn.close()
                conn = None
        self.lateness["ws"] = late

    def lane_sparse(self, j):
        p = self.plan["phase_s"]
        sp = self.plan["sparse"]
        want = sp["request_bytes"]
        target = "/v1/sse?events=%d&size=%d&interval_ms=%d" % (
            sp["events_per_request"], SPARSE_SIZE, SPARSE_EVERY_S * 1000)
        for ph, kind in enumerate(self.plan["phases"]):
            if kind != "S":
                continue
            t = ph * p + j * SPARSE_EVERY_S / SPARSE_STREAMS
            self.wait_until(t)
            conn, cn = None, None
            for r in range(sp["requests_per_phase"]):
                start = self.now()
                arrivals = []
                body = sparse_tag(j, ph, r, sp["events_per_request"])
                try:
                    if conn is None:
                        cn, conn = self.new_conn(self.api, self.proxy_ca)
                    close = r == sp["requests_per_phase"] - 1
                    status, resp = self.request(conn, "POST", target, body, close=close,
                                                on_chunk=lambda _d: arrivals.append(self.now()),
                                                headers=[("x-e00-split-at", str(len(body) // 2))])
                    ok = status == 200 and len(resp) == want and sse_ok(
                        resp, sp["events_per_request"], SPARSE_SIZE, tail=body)
                    self.record(lane="sparse", path="/v1/sse", stream=j, phase=ph, req=r,
                                conn=cn, sched_s=round(t, 3), start_s=round(start, 3),
                                end_s=round(self.now(), 3), ok=ok, sentinel=False,
                                req_len=len(body), req_sha256=sha(body), resp_len=len(resp),
                                resp_sha256=sha(resp), sse_events=sp["events_per_request"] + 1,
                                first_event_s=round(arrivals[0], 3) if arrivals else None)
                    if close:
                        conn.close()
                        conn = None
                except Exception as e:  # noqa: BLE001
                    self.error("sparse %d phase %d req %d" % (j, ph, r), e)
                    self.record(lane="sparse", path="/v1/sse", stream=j, phase=ph, req=r,
                                conn=cn, sched_s=round(t, 3), start_s=round(start, 3),
                                end_s=round(self.now(), 3), ok=False, sentinel=False,
                                req_len=len(body), req_sha256=sha(body), resp_len=None,
                                resp_sha256=None, sse_events=None)
                    if conn:
                        conn.close()
                    conn = None

    def lane_native(self):
        duration = self.plan["duration_s"]
        h = hashlib.sha256()
        written, sentinels, late = 0, 0, []
        fd = os.open(self.native_path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        try:
            for s in range(duration):
                self.wait_until(s)
                late.append(round(self.now() - s, 3))
                for j in range(NATIVE_LINES):
                    line = native_line(s, j)
                    if j == 0 and s % SENTINEL_EVERY == NATIVE_SENTINEL_AT:
                        line = native_line(s, j, sentinel_index=s)
                        cut = line.index(SENTINEL_PREFIX) + 10
                        os.write(fd, line[:cut])
                        time.sleep(0.3)
                        os.write(fd, line[cut:])
                        sentinels += 1
                    else:
                        os.write(fd, line)
                    h.update(line)
                    written += len(line)
        except OSError as e:
            self.error("native", e)
        finally:
            os.close(fd)
        self.native = {"bytes": written, "sha256": h.hexdigest(), "sentinel_lines": sentinels}
        self.lateness["native"] = late

    def stall_trigger(self):
        t = self.plan["stall_at_s"]
        self.wait_until(t)
        start = self.now()
        try:
            cn, conn = self.new_conn(self.api, self.proxy_ca)
            status, resp = self.request(conn, "POST", "/v1/bulk?size=0", close=True)
            conn.close()
            ok = status == 200 and resp == b""
        except Exception as e:  # noqa: BLE001
            self.error("stall trigger", e)
            cn, ok = None, False
        self.stall = {"sched_s": t, "start_s": round(start, 3), "end_s": round(self.now(), 3),
                      "conn": cn, "ok": ok}
        self.record(lane="bulk", path="/v1/bulk", conn=cn, sched_s=t, start_s=round(start, 3),
                    end_s=round(self.now(), 3), ok=ok, sentinel=False, req_len=0,
                    req_sha256=sha(b""), resp_len=0 if ok else None,
                    resp_sha256=sha(b"") if ok else None)

    def auth_request(self):
        start = self.now()
        try:
            cn, conn = self.new_conn(self.auth, self.upstream_ca)
            status, resp = self.request(conn, "POST", "/oauth/token", b"grant_type=e02",
                                        close=True, host=self.auth)
            conn.close()
            ok = status == 200 and resp.startswith(b'{"grant":"e00-synthetic"')
        except Exception as e:  # noqa: BLE001
            self.error("auth", e)
            cn, ok = None, False
        self.record(lane="auth", path="/oauth/token", conn=cn, sched_s=0,
                    start_s=round(start, 3), end_s=round(self.now(), 3), ok=ok, sentinel=False,
                    req_len=len(b"grant_type=e02"), req_sha256=None, resp_len=None,
                    resp_sha256=None)

    def run(self):
        self.t0 = time.monotonic()
        self.t0_wall = time.time()
        self.auth_request()
        threads = [threading.Thread(target=f, name=n) for n, f in (
            ("echo", self.lane_echo), ("sse", self.lane_sse), ("ws", self.lane_ws),
            ("native", self.lane_native), ("stall", self.stall_trigger))]
        threads += [threading.Thread(target=self.lane_sparse, args=(j,), name="sparse%d" % j)
                    for j in range(SPARSE_STREAMS)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        return self.report()

    def report(self):
        ex, ws = self.exchanges, [m for m in self.ws_messages if m["dir"] != "error"]
        sums = {
            "echo_up": sum(e["req_len"] for e in ex if e["lane"] == "echo" and e["ok"]),
            "echo_down": sum(e["resp_len"] for e in ex if e["lane"] == "echo" and e["ok"]),
            "sse_dense_down": sum(e["resp_len"] for e in ex if e["lane"] == "sse" and e["ok"]),
            "sse_sparse_down": sum(e["resp_len"] for e in ex if e["lane"] == "sparse" and e["ok"]),
            "ws_client": sum(m["len"] for m in ws if m["dir"] == "client" and m["ok"]),
            "ws_server": sum(m["len"] for m in ws if m["dir"] == "server" and m["ok"]),
            "native": self.native.get("bytes", 0),
        }
        pl = self.plan
        checks = {
            "all_exchanges_ok": all(e["ok"] for e in ex),
            "all_ws_messages_ok": all(m["ok"] for m in self.ws_messages),
            "dense_bytes": sums["echo_up"] + sums["echo_down"] + sums["sse_dense_down"]
            + sums["ws_client"] + sums["ws_server"] == pl["dense"]["bytes"],
            "sparse_bytes": sums["sse_sparse_down"] == pl["sparse"]["bytes"],
            "native_bytes": sums["native"] == pl["native"]["bytes"],
            "counts": (sum(1 for e in ex if e["lane"] == "echo") == pl["counts"]["echo_exchanges"]
                       and sum(1 for e in ex if e["lane"] in ("sse", "sparse"))
                       == pl["counts"]["sse_exchanges"]
                       and sum(1 for m in ws if m["dir"] == "client")
                       == pl["counts"]["ws_client_messages"]),
            "stall_triggered": bool(self.stall.get("ok")),
            "no_errors": not self.errors,
        }
        late = {k: {"max_s": max(v) if v else None,
                    "over_1s": sum(1 for x in v if x > 1.0), "n": len(v)}
                for k, v in self.lateness.items()}
        return {
            "schema": "e02-early-load/1", "plan": pl, "t0_wall": self.t0_wall,
            "duration_s": round(self.now(), 3), "test_id": self.test_id,
            "ok": all(checks.values()), "checks": checks, "totals": sums,
            "total_bytes": sum(sums.values()), "stall": self.stall, "native": self.native,
            "lateness": late, "lateness_series": self.lateness, "errors": self.errors,
            "exchanges": ex, "ws_messages": self.ws_messages,
        }


def sse_ok(body, events, size, tail=None):
    """`events` endpoint events of `size` pad bytes, then, when `tail` is given, the endpoint's
    echo of the request body (`data: <tail>\n\n`) and nothing else."""
    want = sum(sse_event_len(i, size) for i in range(events))
    if tail is not None:
        want += tag_event_len(len(tail))
    if len(body) != want:
        return False
    pos = 0
    for i in range(events):
        head = b'data: {"i":%d,"pad":"' % i
        if body[pos:pos + len(head)] != head:
            return False
        pad = body[pos + len(head):pos + len(head) + size]
        if not re.fullmatch(rb"[a-z ]+", pad) or body[pos + len(head) + size:pos + len(head) + size + 4] != b'"}\n\n':
            return False
        pos += len(head) + size + 4
    return tail is None or body[pos:] == b"data: " + tail + b"\n\n"


def native_line(s, j, sentinel_index=None):
    head = b'{"e02":"native","s":%d,"j":%d,"pad":"' % (s, j)
    tail = b'"}\n'
    n = NATIVE_LINE_BYTES - len(head) - len(tail)
    pad = filler(30_000_000 + s * NATIVE_LINES + j, n)
    if sentinel_index is not None:
        pad = with_sentinel(pad, 200, sentinel_index)
    return head + pad + tail


# ---------------------------------------------------------------- self-check


def selfcheck():
    fails = []

    def check(name, ok):
        if not ok:
            fails.append(name)

    full = plan("full")
    check("full dense bytes", full["dense"]["bytes"] == 900 * 131072 == 117964800)
    check("full sparse bytes", full["sparse"]["bytes"] == 368640)
    check("full native bytes", full["native"]["bytes"] == 7372800)
    check("full total 119.8828125 MiB", full["total_bytes"] == 125706240
          and full["total_mib"] == 119.8828125)
    check("dense tick 128 KiB", full["dense"]["bytes_per_tick"] == 131072)
    check("dense half split", 2 * ECHO_BYTES + full["dense"]["sse_down"] == 65536
          == 2 * WS_MESSAGES * WS_BYTES)
    check("sparse event 240 B", full["sparse"]["event_bytes"] == 240
          and all(sse_event_len(i, SPARSE_SIZE) == 240 for i in range(10)))
    check("sparse request 10 x 256 B", full["sparse"]["request_bytes"] == 2560
          == 10 * 240 + tag_event_len(full["sparse"]["tag_bytes"]))
    check("smoke sparse request 3 x 256 B", plan("smoke")["sparse"]["request_bytes"] == 768)
    check("full sse events", full["counts"]["sse_events"] == 900 * 9 + 144 * 11)
    check("sparse 30 events per phase", full["sparse"]["requests_per_phase"]
          * full["sparse"]["events_per_request"] == 30)
    check("stall inside the second dense phase", full["phases"][full["stall_at_s"] // 300] == "D"
          and full["stall_at_s"] == 720)
    check("full sentinels", full["sentinels"] == {"echo": 15, "ws": 15, "native": 30})
    smoke = plan("smoke")
    check("smoke stall in dense", smoke["phases"][smoke["stall_at_s"] // smoke["phase_s"]] == "D")
    check("smoke totals", smoke["dense"]["bytes"] == 90 * 131072
          and smoke["sparse"]["bytes"] == 3 * 16 * 3 * 256
          and smoke["native"]["bytes"] == 180 * 4096)
    # The endpoint's SSE event, built independently of sse_event_len.
    ev = b'data: {"i":7,"pad":"' + b"a" * SSE_SIZE + b'"}\n\n'
    check("sse event formula", len(ev) == sse_event_len(7, SSE_SIZE) == 4088)
    tag = sse_tag(7)
    check("dense sse 32 KiB with its tag", 8 * len(ev) + len(b"data: " + tag + b"\n\n") == 32768
          and len(tag) == SSE_TAG_BYTES)
    events = b"".join(b'data: {"i":%d,"pad":"' % i + b"ab c" * 58 + b'"}\n\n' for i in range(10))
    check("sse validator", sse_ok(events, 10, 232) and not sse_ok(b"x" * 2560, 10, 232))
    check("sse validator tail", sse_ok(events + b"data: " + tag + b"\n\n", 10, 232, tail=tag)
          and not sse_ok(events + b"data: " + sse_tag(8) + b"\n\n", 10, 232, tail=tag)
          and not sse_ok(events, 10, 232, tail=tag))
    # F-16: every body has its own digest, so a lost one is identifiable by sha256 alone.
    ticks = full["dense"]["ticks"]
    sp = full["sparse"]
    sparse_tags = [sparse_tag(j, ph, r, sp["events_per_request"])
                   for ph, kind in enumerate(full["phases"]) if kind == "S"
                   for j in range(SPARSE_STREAMS) for r in range(sp["requests_per_phase"])]
    check("sparse tags unique and sized", len(set(sparse_tags)) == len(sparse_tags) == 144
          and all(len(x) == sp["tag_bytes"] == 152 for x in sparse_tags))
    dense_tags = [sse_tag(k) for k in range(ticks)]
    check("dense sse tags unique", len(set(dense_tags)) == ticks
          and not set(dense_tags) & set(sparse_tags))
    check("echo bodies unique", len({sha(echo_body(k)) for k in range(ticks)}) == ticks)
    check("ws payloads unique", len({sha(ws_payload(k, m)) for k in range(ticks)
                                     for m in range(WS_MESSAGES)}) == ticks * WS_MESSAGES)
    check("tags are filler-safe", all(b"\n" not in x and re.fullmatch(rb"[a-z0-9 ]+", x)
                                      for x in dense_tags[:3] + sparse_tags[:3]
                                      + [echo_body(1)[:40], ws_payload(1, 1)[:40]]))
    # Filler: nothing a redactor would take for a secret.
    runs = re.findall(rb"[A-Za-z0-9_-]+", POOL)
    check("filler words short", max(len(r) for r in runs) <= 9 and b"@" not in POOL)
    check("filler exact size", len(filler(12345, ECHO_BYTES)) == ECHO_BYTES)
    # Sentinel shape and placement.
    s = sentinel(30)
    check("sentinel matches anthropic_key", re.fullmatch(rb"sk-ant-[A-Za-z0-9_\-]{20,}", s) is not None)
    b = with_sentinel(echo_body(30), 8001, 30)
    check("echo sentinel keeps size", len(b) == ECHO_BYTES and b[8001:8001 + len(s)] == s)
    check("echo chunk split inside sentinel", 8001 < 8011 < 8001 + len(s))
    w = with_sentinel(ws_payload(30, 0), WS_FRAG - 20, 30)
    check("ws sentinel straddles a frame", len(w) == WS_BYTES
          and w.index(s) < WS_FRAG < w.index(s) + len(s))
    nl = native_line(45, 0, sentinel_index=45)
    check("native line exact", len(nl) == NATIVE_LINE_BYTES == len(native_line(3, 2))
          and nl.endswith(b"\n") and json.loads(nl)["s"] == 45)
    s45 = sentinel(45)
    check("native sentinel split inside", nl.index(s45) < nl.index(SENTINEL_PREFIX) + 10
          < nl.index(s45) + len(s45))
    check("report never holds the sentinel", SENTINEL_PREFIX not in json.dumps(
        {"k": sha(b)}).encode())
    # WebSocket framing round trip: masked fragmented client message with an interleaved
    # ping, and unmasked server frames of every length form.
    import io
    rng = Rng(7)
    msg = filler(3, WS_BYTES)
    rf = io.BufferedReader(io.BytesIO(ws_client_message(msg, WS_FRAG, rng)))
    pings = []
    op, got, frames = ws_read_message(rf, lambda o, d: pings.append((o, d)))
    check("ws client round trip", op == 1 and got == msg and frames == 4 and pings == [(9, b"e02")])
    for n in (5, 300, 70000):
        data = filler(4, n)
        rf = io.BufferedReader(io.BytesIO(ws_frame(True, 2, data)))
        check("ws server frame %d" % n, ws_read_frame(rf) == (True, 2, data))
    # Chunked request encoding and the proxy URL.
    chunks = [b[:8011], b[8011:]]
    enc = b"".join(b"%x\r\n" % len(c) + c + b"\r\n" for c in chunks) + b"0\r\n\r\n"

    class F:
        def __init__(self, data):
            self.rf = io.BufferedReader(io.BytesIO(data))
    check("chunked round trip", Conn.body(F(enc), {"transfer-encoding": "chunked"}) == b)
    px = Proxy("http://goalcup:s3cr3t@127.0.0.1:4321")
    check("proxy parse", (px.host, px.port) == ("127.0.0.1", 4321) and "s3cr3t" not in repr(px))
    print(json.dumps({"ok": not fails, "failures": fails,
                      "full_total_bytes": full["total_bytes"], "full_total_mib": full["total_mib"],
                      "smoke_total_bytes": smoke["total_bytes"]}, indent=1))
    return 0 if not fails else 1


def main(argv):
    if argv[:1] == ["selfcheck"]:
        return selfcheck()
    if argv[:1] == ["plan"]:
        name = argv[2] if len(argv) == 3 and argv[1] == "--schedule" else "full"
        if name not in SCHEDULES:
            return 64
        print(json.dumps(plan(name), indent=1, sort_keys=True))
        return 0
    if argv[:1] != ["run"] or len(argv) % 2 != 1:
        sys.stderr.write(__doc__)
        return 64
    args = dict(zip(argv[1::2], argv[2::2]))
    need = {"--schedule", "--endpoint", "--auth-endpoint", "--upstream-ca", "--native-path",
            "--test-id", "--report"}
    if set(args) != need or args["--schedule"] not in SCHEDULES:
        sys.stderr.write(__doc__)
        return 64
    try:
        load = Load(args, args["--schedule"])
        report = load.run()
    except (KeyError, ValueError, OSError) as e:
        sys.stderr.write("early-load: cannot run: %s\n" % type(e).__name__)
        return 3
    with open(args["--report"], "w", encoding="utf-8") as fh:
        json.dump(report, fh)
    sys.stdout.write(json.dumps({"ok": report["ok"], "checks": report["checks"],
                                 "total_bytes": report["total_bytes"],
                                 "duration_s": report["duration_s"],
                                 "errors": len(report["errors"])}) + "\n")
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
