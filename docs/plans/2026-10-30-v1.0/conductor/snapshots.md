# Sandbox snapshots

Written 2 November 2026, 01:15 UTC, from `node box.mjs snaps` on the old machine. The full listing had 2590 snapshots, nearly all one-off pad copies that expire on their own 30 days after creation (the earliest on 27 November 2026). Only the ones a new session needs are kept here. Run `node box.mjs snaps` for the live list; it marks the build snapshot `[build]` and the pad image `[pad]` when `build-snapshot.id` and `$GC_REPO/scripts/pad/image.id` are readable.

| Role | Snapshot id | Created | Expires | Size |
|---|---|---|---|---|
| Build snapshot (Rust, nextest, zstd, Node 22, Python 3.9). Id also in `build-snapshot.id` | `snap_5V1QbpB1wApS4scewdsrqPk2moyd` | 2026-10-31 08:03Z | never | 1.04 GB |
| Pad image (the harness CLIs; `scripts/pad/image.id`) | `snap_j5dV4pLEm06EUwm1E47wDXFJiA48` | 2026-11-01 06:27Z | never | 5.47 GB |
| Older snapshot with no expiry, role not recorded | `snap_Nsdum3e3yeMCzWDKSSVE2nct4kgw` | 2026-10-30 10:52Z | never | 1.91 GB |
| Older snapshot with no expiry, role not recorded | `snap_doHYY2YaOLhkmu50oEYeaftPvqpx` | 2026-10-30 10:33Z | never | 1.05 GB |

Boxes at that time: 2 running, of 2592 listed (the count includes stopped ones).

If a listed snapshot is gone, rebuild the build snapshot with `node box.mjs snapshot-build`. The pad image is rebuilt by the pad's own image script (see `scripts/pad`).
