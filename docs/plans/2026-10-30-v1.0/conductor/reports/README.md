# Reports

Copies of the `REPORT.txt` files that came out of the pad and investigation folders (scrubbed, then checked with `keyscan.py`). Each row is read from the first lines of that report. "Head" is the integration commit the report names. Read the file for the numbers; this table only says which to open.

| Report | Head | What it shows |
|---|---|---|
| `c4.txt` | `cee85bdc` | Refetch of 12 staging runs to check that the server copy equals the local copy (C4). |
| `full1.txt` | `27341339` | First `full` pad rows on the post-cut head. |
| `full2.txt` | `78618a77` | Second set of `full` pad rows after the verifier and rig fixes. |
| `full3.txt` | `2702551e` | Third set of `full` pad rows. |
| `full4.txt` | `90a07d78` | Hermes rows. |
| `gw2.txt` | `a31a45e4` | Gateway rows (rung 7, gateway case) on the pinned cheap model, with a local model patch. |
| `gw3.txt` | `a31a45e4` | Gateway rows rerun after the gateway team got paid credit; the gateway works on four harness rows. |
| `hermes-parent.txt` | `a31a45e4` | Investigation: Hermes subagent parent links without `state.db`. |
| `history-warn.txt` | `61ef1156` | Why `history_complete` always warned on Claude Code rows, and the fix. |
| `rung1x.txt` | `0bf25314` | Rung 1x: micro run, server off, for every harness beyond Claude Code and Codex. |
| `rung2.txt` | `8dc99969` | Rung 1 again and rung 2. |
| `rung2b.txt` | `1455cd37` | Rung 1 recheck and rung 2 after the rig fixes. |
| `rung3b.txt` | `f2f96f0e` | Rung 3 (subagents), Claude Code and Codex. |
| `rung5.txt` | `6fb5db63` | Rung 5 (watchdog kill and `goalcup recover`), plus kill-both and `goalcup seal` on a dead entry. |
| `rung7.txt` | `5825d6d8` | Rungs 7 (gateway), 4 (recorder crash, rerun) and 8 (dodger, rerun). |
| `rungs.txt` | `611d7a7d` | Rungs 3, 4, 6, 8 and 9 on Claude Code and Codex. |
| `sweep2.txt` | `cafcdef4` | Second re-sweep of the test pad. |

Other files here: `STATUS-REPORT.md` and `STOCKTAKE.md` (the position at the start of the finish), `WRITING-PLANS.md` (how the four plans were to be written), `review-latest.md`, `reviews/`, the two `review-*` plan review files, and `next-pad-verify.md` (what to check on the next pad run).
