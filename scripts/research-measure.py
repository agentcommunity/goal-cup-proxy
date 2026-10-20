#!/usr/bin/env python3
"""Measure chain parts by size, kind, stream, quiet gaps and compression; aggregates only.

WP-0 of the event readiness plan. Reads each TRACE_DIR's chain parts (`chain.jsonl`, then
`chain.<20 digits>.jsonl` by start offset, D55) as one stream, line by line. Prints counts,
byte sizes and durations only: never record text, payload bytes, ids, labels or paths.
"""

import argparse
import array
import bisect
import calendar
import json
import lzma
import math
import os
import re
import shutil
import subprocess
import sys
import threading
import zlib


# The writer's part grammar (`goalcup_platform::tracecap::parse_part_name`).
PART_RE = re.compile(r"chain\.([0-9]{20})\.jsonl")
# A v5 line is at most 128 KiB; anything longer is read in pieces and counted unparsed.
LINE_LIMIT = 1 << 20
CHUNK = 1 << 20
GAP_THRESHOLDS = (1, 5, 15, 30, 60, 300)
# Words the output may carry: closed-set kinds, sources, reasons and connector hints.
SAFE_WORD = re.compile(r"[a-z0-9][a-z0-9._-]{0,63}")
TIME_RE = re.compile(
    r"(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)(\.\d+)?(Z|[+-]\d\d:\d\d)")
GEN_RE = re.compile(r"#g[0-9]+$")
TOP_NATIVE_FILES = 10


def safe(word, fallback="other"):
    return word if isinstance(word, str) and SAFE_WORD.fullmatch(word) else fallback


def parse_time(text):
    """RFC 3339 to unix seconds, or None. Python 3.9's fromisoformat refuses `Z`."""
    if not isinstance(text, str):
        return None
    match = TIME_RE.fullmatch(text)
    if not match:
        return None
    y, mo, d, h, mi, s, frac, zone = match.groups()
    try:
        value = calendar.timegm((int(y), int(mo), int(d), int(h), int(mi), int(s)))
    except (ValueError, OverflowError):
        return None
    if frac:
        value += float(frac)
    if zone != "Z":
        sign = 1 if zone[0] == "+" else -1
        value -= sign * (int(zone[1:3]) * 3600 + int(zone[4:6]) * 60)
    return value


def b64_decoded_len(text):
    if not isinstance(text, str):
        return 0
    n = len(text)
    pad = len(text) - len(text.rstrip("="))
    return max(0, n * 3 // 4 - pad)


def list_parts(trace_dir):
    """[(start, path, size)] sorted by start, as `export::chain_parts` orders them."""
    parts = []
    for name in os.listdir(trace_dir):
        if name == "chain.jsonl":
            start = 0
        else:
            match = PART_RE.fullmatch(name)
            if not match or int(match.group(1)) == 0:
                continue
            start = int(match.group(1))
        path = os.path.join(trace_dir, name)
        if not os.path.isfile(path) or os.path.islink(path):
            continue
        parts.append((start, path, os.path.getsize(path)))
    parts.sort()
    return parts


def iter_lines(parts, feed=None):
    """Yield (line or None, size, complete) across parts; `feed` sees every byte once.

    A line over LINE_LIMIT is read in pieces and yielded once as None: counted, never parsed.
    """
    carry, oversize = b"", 0
    for _start, path, _size in parts:
        with open(path, "rb") as stream:
            while True:
                piece = stream.readline(LINE_LIMIT)
                if not piece:
                    break
                if feed is not None:
                    feed(piece)
                ended = piece.endswith(b"\n")
                if oversize or len(carry) + len(piece) > LINE_LIMIT:
                    oversize += len(carry) + len(piece)
                    carry = b""
                    if ended:
                        yield None, oversize, True
                        oversize = 0
                    continue
                line, carry = carry + piece, b""
                if ended:
                    yield line, len(line), True
                else:
                    carry = line  # a part that ends mid-line continues in the next
    if oversize:
        yield None, oversize, False
    elif carry:
        yield carry, len(carry), False


def percentile(sorted_values, zeros, fraction):
    total = zeros + len(sorted_values)
    if total == 0:
        return None
    rank = min(total - 1, max(0, math.ceil(fraction * total) - 1))
    if rank < zeros:
        return 0.0
    return sorted_values[rank - zeros]


class Compressor:
    """Byte counter over a compressor fed in order; never keeps more than a chunk."""

    def __init__(self, label, argv=None, make=None):
        self.label, self.argv, self.out_bytes, self.error = label, argv, 0, None
        self.buffer, self.obj, self.proc = [], None, None
        self.buffered = 0
        if argv:
            try:
                self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                             stderr=subprocess.DEVNULL)
            except OSError:
                self.error = "could not start"
                return
            self.drain = threading.Thread(target=self._drain, daemon=True)
            self.drain.start()
        else:
            self.obj = make()

    def _drain(self):
        while True:
            block = self.proc.stdout.read(CHUNK)
            if not block:
                return
            self.out_bytes += len(block)

    def feed(self, data):
        self.buffer.append(data)
        self.buffered += len(data)
        if self.buffered >= CHUNK:
            self._flush()

    def _flush(self):
        data = b"".join(self.buffer)
        self.buffer, self.buffered = [], 0
        if self.error or not data:
            return
        if self.proc is not None:
            try:
                self.proc.stdin.write(data)
            except (BrokenPipeError, OSError):
                self.error = "compressor stopped"
        else:
            self.out_bytes += len(self.obj.compress(data))

    def finish(self):
        self._flush()
        if self.argv and self.proc is None:
            return None
        if self.proc is not None:
            try:
                self.proc.stdin.close()
            except OSError:
                pass
            code = self.proc.wait()
            self.drain.join()
            if code != 0 and not self.error:
                self.error = "exit %d" % code
        else:
            self.out_bytes += len(self.obj.flush())
        return None if self.error else self.out_bytes


def compressors(enabled):
    if not enabled:
        return "off", []
    zstd = shutil.which("zstd")
    if zstd:
        return "zstd", [
            Compressor("zstd -3", [zstd, "-q", "-c", "-3", "-T1"]),
            Compressor("zstd -3 --long=27", [zstd, "-q", "-c", "-3", "-T1", "--long=27"]),
        ]
    return "fallback", [
        Compressor("zlib -6 (rough proxy, zstd absent)", make=lambda: zlib.compressobj(6)),
        Compressor("lzma -1 (rough proxy, zstd absent)",
                   make=lambda: lzma.LZMACompressor(preset=1)),
    ]


def new_bucket():
    return {"records": 0, "bytes": 0}


def measure(trace_dir, compress=True):
    parts = list_parts(trace_dir)
    contiguous = all(a[0] + a[2] == b[0] for a, b in zip(parts, parts[1:]))
    mode, comps = compressors(compress)

    def feed(data):
        for comp in comps:
            comp.feed(data)

    kinds, gap_reasons, cats = {}, {}, {}
    streams = {}  # (producer, stream) -> [category, native file index or None, decoded bytes]
    native_files, native_gens = {}, {}  # base label -> index; index -> [(bytes, end)]
    hours = {}
    total = {"records": 0, "bytes": 0, "unparsed": 0, "unparsed_bytes": 0, "torn_tail": 0}
    first_t = last_t = prev_t = None
    first_seq = last_seq = None
    zeros, negative = 0, 0
    gaps = array.array("d")

    def cat(name):
        if name not in cats:
            cats[name] = {"streams": 0, "records": 0, "bytes": 0, "segments": 0,
                          "segment_bytes": 0, "b64_bytes": 0, "decoded_bytes": 0}
        return cats[name]

    for line, size, complete in iter_lines(parts, feed):
        total["bytes"] += size
        envelope = None
        if complete and line is not None:
            try:
                envelope = json.loads(line)
            except ValueError:
                envelope = None
        if not isinstance(envelope, dict):
            total["unparsed"] += 1
            total["unparsed_bytes"] += size
            if not complete:
                total["torn_tail"] += 1
            continue
        total["records"] += 1
        seq = envelope.get("seq")
        if isinstance(seq, int):
            first_seq = seq if first_seq is None else first_seq
            last_seq = seq
        t = parse_time(envelope.get("recorded_at"))
        if t is not None:
            first_t = t if first_t is None else min(first_t, t)
            last_t = t if last_t is None else max(last_t, t)
            hour = int(t // 3600)
            hours[hour] = hours.get(hour, 0) + size
            if prev_t is not None:
                delta = t - prev_t
                if delta < 0:
                    negative += 1
                    zeros += 1
                elif delta == 0:
                    zeros += 1
                else:
                    gaps.append(delta)
            prev_t = t
        body = envelope.get("body") if isinstance(envelope.get("body"), dict) else {}
        kind = safe(body.get("kind"))
        record = None
        if kind == "capture":
            content = body.get("content")
            record = content.get("record") if isinstance(content, dict) else None
            if isinstance(record, dict):
                kind = safe(record.get("kind"))
                if kind == "connection" and isinstance(record.get("event"), dict):
                    kind = "connection." + safe(record["event"].get("kind"))
                if kind == "gap":
                    reason = safe(record.get("reason"))
                    gap_reasons[reason] = gap_reasons.get(reason, 0) + 1
            else:
                record = None
        bucket = kinds.setdefault(kind, new_bucket())
        bucket["records"] += 1
        bucket["bytes"] += size
        key = None
        if record is not None and isinstance(record.get("stream"), dict):
            ref = record["stream"]
            key = (str(ref.get("producer")), str(ref.get("stream")))
        if key is None:
            name = "(no stream)"
            entry = None
        else:
            entry = streams.get(key)
            if kind == "open":
                meta = record.get("metadata") if isinstance(record.get("metadata"), dict) else {}
                source = safe(record.get("source"))
                hint = meta.get("protocol_hint")
                name = source if hint is None else source + "/" + safe(hint)
                native = None
                if source == "native":
                    base = GEN_RE.sub("", str(meta.get("label")))
                    native = native_files.setdefault(base, len(native_files))
                    native_gens.setdefault(native, [])
                entry = [name, native, 0]
                streams[key] = entry
                cat(name)["streams"] += 1
            name = entry[0] if entry else "(stream not opened here)"
        c = cat(name)
        c["records"] += 1
        c["bytes"] += size
        if kind == "segment":
            data = record.get("data_b64")
            decoded = b64_decoded_len(data)
            c["segments"] += 1
            c["segment_bytes"] += size
            c["b64_bytes"] += len(data) if isinstance(data, str) else 0
            c["decoded_bytes"] += decoded
            if entry:
                entry[2] += decoded
        elif kind == "end" and entry is not None:
            if entry[1] is not None:
                native_gens[entry[1]].append((entry[2], safe(record.get("reason"))))
            del streams[key]

    for entry in streams.values():
        if entry[1] is not None:
            native_gens[entry[1]].append((entry[2], "open"))

    comp_out = []
    for comp in comps:
        out = comp.finish()
        comp_out.append({
            "method": comp.label,
            "compressed_bytes": out,
            "ratio": round(total["bytes"] / out, 3) if out else None,
            "error": comp.error,
        })

    span = (last_t - first_t) if first_t is not None else None
    sorted_gaps = sorted(gaps)
    n_gaps = zeros + len(sorted_gaps)
    quiet = []
    for threshold in GAP_THRESHOLDS:
        over = sorted_gaps[bisect.bisect_left(sorted_gaps, threshold):]
        quiet.append({"at_least_s": threshold, "count": len(over),
                      "seconds_total": round(sum(over), 3)})
    for bucket in kinds.values():
        bucket["share"] = round(bucket["bytes"] / total["bytes"], 4) if total["bytes"] else 0
    for c in cats.values():
        c["envelope_bytes"] = c["bytes"] - c["decoded_bytes"]
        c["segment_overhead_bytes"] = c["segment_bytes"] - c["decoded_bytes"]
        c["segment_bytes_per_payload_byte"] = (
            round(c["segment_bytes"] / c["decoded_bytes"], 3) if c["decoded_bytes"] else None)
    seg_bytes = sum(c["segment_bytes"] for c in cats.values())
    decoded = sum(c["decoded_bytes"] for c in cats.values())

    files = []
    for index, gens in native_gens.items():
        sizes = [g[0] for g in gens]
        if not sizes:
            continue
        files.append({
            "file": index + 1,
            "generations": len(sizes),
            "ended_complete": sum(1 for g in gens if g[1] == "complete"),
            "total_bytes": sum(sizes),
            "first_gen_bytes": sizes[0],
            "last_gen_bytes": sizes[-1],
            "max_gen_bytes": max(sizes),
            "bytes_per_generation": sizes if len(sizes) <= 50 else None,
            # total / largest: about generations / 2 when each generation re-sends the last.
            "total_over_max": round(sum(sizes) / max(sizes), 2) if max(sizes) else None,
        })
    multi = [f for f in files if f["generations"] > 1]
    multi.sort(key=lambda f: -f["total_bytes"])

    return {
        "parts": len(parts),
        "parts_contiguous": contiguous,
        "first_part_start": parts[0][0] if parts else None,
        "records": total["records"],
        "bytes": total["bytes"],
        "unparsed_lines": total["unparsed"],
        "unparsed_bytes": total["unparsed_bytes"],
        "unterminated_lines": total["torn_tail"],
        "first_seq": first_seq,
        "last_seq": last_seq,
        "wall_span_s": round(span, 3) if span is not None else None,
        "bytes_per_hour": round(total["bytes"] * 3600 / span) if span else None,
        "peak_hour_bytes": max(hours.values()) if hours else None,
        "kinds": dict(sorted(kinds.items(), key=lambda kv: -kv[1]["bytes"])),
        "gap_reasons": dict(sorted(gap_reasons.items())),
        "streams": dict(sorted(cats.items(), key=lambda kv: -kv[1]["bytes"])),
        "segments": {"line_bytes": seg_bytes, "decoded_bytes": decoded,
                     "envelope_bytes": seg_bytes - decoded,
                     "line_bytes_per_payload_byte":
                         round(seg_bytes / decoded, 3) if decoded else None},
        "quiet_gaps": {
            "intervals": n_gaps,
            "zero": zeros,
            "clock_went_back": negative,
            "p50_s": percentile(sorted_gaps, zeros, 0.50),
            "p90_s": percentile(sorted_gaps, zeros, 0.90),
            "p99_s": percentile(sorted_gaps, zeros, 0.99),
            "max_s": sorted_gaps[-1] if sorted_gaps else (0.0 if n_gaps else None),
            "thresholds": quiet,
        },
        "compression": {"mode": mode, "results": comp_out},
        "native": {
            "files": len(files),
            "multi_generation_files": len(multi),
            "generations": sum(f["generations"] for f in files),
            "top_multi_generation": multi[:TOP_NATIVE_FILES],
        },
    }


def human(n):
    if n is None:
        return "-"
    value = float(n)
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if abs(value) < 1024 or unit == "TiB":
            return ("%d %s" % (value, unit)) if unit == "B" else ("%.1f %s" % (value, unit))
        value /= 1024


def secs(value):
    return "-" if value is None else ("%.3f s" % value)


def render(index, r, out):
    w = out.write
    w("== trace %d\n" % index)
    w("parts %d (contiguous: %s, first part start %s)\n"
      % (r["parts"], "yes" if r["parts_contiguous"] else "NO", r["first_part_start"]))
    w("records %d, bytes %s (%d), seq %s..%s\n"
      % (r["records"], human(r["bytes"]), r["bytes"], r["first_seq"], r["last_seq"]))
    if r["unparsed_lines"]:
        w("unparsed lines %d (%s), unterminated %d\n"
          % (r["unparsed_lines"], human(r["unparsed_bytes"]), r["unterminated_lines"]))
    w("wall span %s, bytes/hour %s, peak clock hour %s\n"
      % (secs(r["wall_span_s"]), human(r["bytes_per_hour"]), human(r["peak_hour_bytes"])))
    w("\n%-34s %10s %12s %7s\n" % ("record kind", "count", "bytes", "share"))
    for kind, b in r["kinds"].items():
        w("%-34s %10d %12s %6.1f%%\n" % (kind, b["records"], human(b["bytes"]), 100 * b["share"]))
    if r["gap_reasons"]:
        w("gap reasons: %s\n" % ", ".join("%s %d" % kv for kv in r["gap_reasons"].items()))
    w("\n%-40s %7s %9s %11s %9s %11s %11s %7s\n" % (
        "stream (source/hint)", "streams", "records", "bytes", "segments", "decoded",
        "envelope", "x/pay"))
    for name, c in r["streams"].items():
        ratio = c["segment_bytes_per_payload_byte"]
        w("%-40s %7d %9d %11s %9d %11s %11s %7s\n" % (
            name, c["streams"], c["records"], human(c["bytes"]), c["segments"],
            human(c["decoded_bytes"]), human(c["envelope_bytes"]),
            "-" if ratio is None else "%.2f" % ratio))
    s = r["segments"]
    w("segments: line %s, payload %s, envelope %s, line bytes per payload byte %s\n" % (
        human(s["line_bytes"]), human(s["decoded_bytes"]), human(s["envelope_bytes"]),
        s["line_bytes_per_payload_byte"]))
    q = r["quiet_gaps"]
    w("\nquiet gaps between records: %d intervals (%d zero, %d clock went back)\n"
      % (q["intervals"], q["zero"], q["clock_went_back"]))
    w("p50 %s  p90 %s  p99 %s  max %s\n"
      % (secs(q["p50_s"]), secs(q["p90_s"]), secs(q["p99_s"]), secs(q["max_s"])))
    for t in q["thresholds"]:
        w("  >= %4d s: %8d gaps, %12.1f s quiet in total\n"
          % (t["at_least_s"], t["count"], t["seconds_total"]))
    c = r["compression"]
    w("\ncompression (%s):\n" % c["mode"])
    if c["mode"] == "off":
        w("  skipped\n")
    if c["mode"] == "fallback":
        w("  zstd not found on PATH; figures below are a rough proxy only\n")
    for res in c["results"]:
        w("  %-40s %12s  ratio %s%s\n" % (
            res["method"], human(res["compressed_bytes"]), res["ratio"],
            "  (failed: %s)" % res["error"] if res["error"] else ""))
    n = r["native"]
    w("\nnative: %d files, %d generations, %d files with more than one generation\n"
      % (n["files"], n["generations"], n["multi_generation_files"]))
    for f in n["top_multi_generation"]:
        w("  file %d: %d generations (%d ended complete), total %s, first %s, last %s, "
          "max %s, total/max %s\n" % (
              f["file"], f["generations"], f["ended_complete"], human(f["total_bytes"]),
              human(f["first_gen_bytes"]), human(f["last_gen_bytes"]),
              human(f["max_gen_bytes"]), f["total_over_max"]))
        if f["bytes_per_generation"]:
            w("    bytes per generation: %s\n" % " ".join(str(b) for b in f["bytes_per_generation"]))
    w("\n")


def main(argv=None, out=None):
    out = out or sys.stdout
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace_dirs", nargs="+", metavar="TRACE_DIR")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    parser.add_argument("--no-compress", action="store_true", help="skip the compression pass")
    args = parser.parse_args(argv)
    reports, status = [], 0
    for index, trace_dir in enumerate(args.trace_dirs, 1):
        # Traces are numbered by argument order; their paths never reach the output.
        if not os.path.isdir(trace_dir):
            reports.append({"trace": index, "error": "not a directory"})
            status = 1
            continue
        try:
            report = measure(trace_dir, compress=not args.no_compress)
        except OSError as error:
            reports.append({"trace": index, "error": type(error).__name__})
            status = 1
            continue
        if not report["parts"]:
            status = 1
        report["trace"] = index
        reports.append(report)
    if args.json:
        json.dump({"traces": reports}, out, indent=1, sort_keys=True)
        out.write("\n")
    else:
        for report in reports:
            if "error" in report:
                out.write("== trace %d\nerror: %s\n\n" % (report["trace"], report["error"]))
            else:
                render(report["trace"], report, out)
    out.flush()
    return status


if __name__ == "__main__":
    sys.exit(main())
