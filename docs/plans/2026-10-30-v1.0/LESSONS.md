# LESSONS: PRD-17 status table

The one map from each lesson in [PRD-17](PRD-17-lessons-old-code.md) sections 2.1 to 2.16 to its
guard. The case text of each row lives only in PRD-17 and is not copied here. Owned and kept by the
lessons lane (PRD-17); read by the deletion gate (PRD-17 Acceptance, checks 1 to 4).

- **Ids.** `L-<id>` here (`L-H2`); `PRD-17 <id>` everywhere else. The ledger and other PRDs reuse
  letters such as H2 with other meanings.
- **Guard codes.** `A:` an automated test, as `file::test` or as a test file name (Rust under
  `crates/goalcup/tests/`, or a Worker test under `server/worker/test/`). `R:` a real-run check: a pad
  row (PRD-12) or RR1 to RR6 below. `D:` a data or docs file plus the key or section. `X:` the owner
  decision or PRD that dropped or inverted the case. Entries are separated by `; `; the owner PRD of a
  test stands in parentheses after it. `see L-<id>` reuses that row's guard. `(name pending)` marks a
  case PRD-17 asked its owner to name (night log).
- **Status.** `open` (not yet shown), `green` (every `A:` guard passes on integration in a build box),
  `real-run pass` (every `R:` guard shown in a pad run), `dropped: <decision>`. Every row starts `open`;
  rows whose only guard is `X:` stay `open` until the lane flips them at the gate.
- **Commit.** The integration commit that made the row `green`, or `-`.
- **A renamed guard** is fixed by the lessons lane with an edit to this file and a night-log line. It
  is never a gate failure; only a lesson with no guard at all fails check 1.

## 2.1 CONNECT and admission (C)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-C1 | A: lessons_connect::connect_and_client_hello_in_one_write_are_decrypted | PRD-17 | open | - |
| L-C2 | A: lessons_connect::bad_heads_get_fixed_answers_without_echo | PRD-17 | open | - |
| L-C3 | A: lessons_connect::authority_forms | PRD-17 | open | - |
| L-C4 | A: lessons_connect::client_hello_in_two_records_and_one_byte_reads_decides_by_sni | PRD-17 | open | - |
| L-C5 | A: lessons_connect::close_before_hello_opens_no_upstream | PRD-17 | open | - |
| L-C6 | A: lessons_connect::unlisted_refusal_is_one_round; A: lessons_connect::listed_refusal_is_retried; A: proxy_connect.rs (PRD-03, the reset rule) | PRD-17 | open | - |
| L-C7 | R: RR1; A: lessons_connect::idle_tunnel_survives_120s (full tier) | PRD-17 | open | - |
| L-C8 | A: lessons_connect::one_bad_handshake_leaves_others_relaying | PRD-17 | open | - |
| L-C9 | A: proxy_connect.rs (PRD-03) | PRD-03 | open | - |
| L-C10 | A: proxy_connect.rs (PRD-03); A: lessons_connect::listed_sni_to_a_local_fake_server_is_never_trusted | PRD-03, PRD-17 | open | - |
| L-C11 | A: proxy_connect.rs (PRD-03); A: watchdog_rlimit.rs (PRD-02) | PRD-03 | open | - |
| L-C12 | A: watchdog_restart.rs (PRD-02) | PRD-02 | open | - |

## 2.2 TLS and the run CA (T)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-T1 | A: lessons_tls::leaf_aki_names_the_ca_key; R: RR3 | PRD-17 | open | - |
| L-T2 | A: lessons_tls::ca_constraints_validity_and_leaf_shape | PRD-17 | open | - |
| L-T3 | A: proxy_tls.rs (PRD-03); A: lessons_tls::untrusted_upstream_reaches_client_unchanged | PRD-03, PRD-17 | open | - |
| L-T4 | A: lessons_tls::ca_files_are_new_0600_and_never_follow_a_link | PRD-17 | open | - |
| L-T5 | A: lessons_tls::bundle_*; R: RR5 | PRD-17 | open | - |
| L-T6 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-T7 | A: proxy_tls.rs (PRD-03, ALPN mirror matrix) | PRD-03 | open | - |
| L-T8 | A: proxy_tls.rs (PRD-03) | PRD-03 | open | - |
| L-T9 | D: docs/PARTICIPANT-GUIDE.md known limits (PRD-15) | PRD-03, PRD-15 | open | - |

## 2.3 HTTP/1.1 framing (H)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-H1 | A: lessons_http1::head_scan_skips_blank_lines_and_takes_bare_lf | PRD-17 | open | - |
| L-H2 | A: lessons_http1::unframeable_heads_relay_on_inside_a_named_loss; A: proxy_http1.rs (PRD-03, generic fault case) | PRD-17 | open | - |
| L-H3 | A: lessons_http1::transfer_gzip_then_chunked_decodes | PRD-17 | open | - |
| L-H4 | A: lessons_http1::chunked_split_at_every_byte_ends_exactly; A: lessons_http1::chunk_size_rules; A: proxy_http1.rs (PRD-03, trailers) | PRD-17 | open | - |
| L-H5 | A: proxy_http1.rs (PRD-03); A: lessons_http1::framing_matrix | PRD-03, PRD-17 | open | - |
| L-H6 | A: proxy_http1.rs (PRD-03); A: lessons_http1::informational_then_final_keeps_the_pair | PRD-03, PRD-17 | open | - |
| L-H7 | A: proxy_http1.rs (PRD-03) | PRD-03 | open | - |
| L-H8 | A: lessons_http1::eof_inside_a_body_is_interrupted; A: lessons_http1::request_after_upstream_close_is_marked_unsent | PRD-17 | open | - |
| L-H9 | A: lessons_http1::head_over_64k_relays_inside_a_named_loss | PRD-17 | open | - |
| L-H10 | A: lessons_http1::authorization_forwarded_unchanged_and_starred_in_meta | PRD-17 | open | - |
| L-H11 | A: PRD-03 rule 2 tests (file pending) | PRD-03 | open | - |
| L-H12 | X: PRD-03 rule 2 | PRD-03 | open | - |

## 2.4 Body decoding (D)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-D1 | A: proxy_http1.rs (PRD-03); A: lessons_decode::brotli_whole_32k_pieces_and_trailing_bytes; R: RR2 | PRD-03, PRD-17 | open | - |
| L-D2 | A: lessons_decode::zstd_frame_ending_on_the_buffer_edge_is_complete; A: lessons_decode::zstd_cut_after_whole_pieces_is_a_gap; R: a pad run with a ChatGPT sign-in, else not shown in a real run | PRD-17 | open | - |
| L-D3 | A: lessons_decode::gzip_two_members_and_a_bad_crc | PRD-17 | open | - |
| L-D4 | A: lessons_decode::raw_and_zlib_deflate_split_after_byte_one | PRD-17 | open | - |
| L-D5 | A: proxy_http1.rs (PRD-03) | PRD-03 | open | - |
| L-D6 | A: lessons_decode::corrupt_tail_keeps_the_prefix | PRD-17 | open | - |
| L-D7 | A: lessons_decode::bomb_relays_on_in_bounded_pieces | PRD-17 | open | - |
| L-D8 | A: lessons_decode.rs | PRD-17 | open | - |

## 2.5 WebSocket and upgrade (W)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-W1 | A: proxy_ws.rs (PRD-03); R: RR1 | PRD-03 | open | - |
| L-W2 | A: lessons_ws::negotiation_accepts_only_rfc7692; A: lessons_ws::unknown_extension_relays_inside_a_named_loss | PRD-17 | open | - |
| L-W3 | A: lessons_ws::server_max_window_bits_10_decodes | PRD-17 | open | - |
| L-W4 | A: lessons_ws::guard_drop_keeps_the_next_message_decodable | PRD-17 | open | - |
| L-W5 | A: lessons_ws::bfinal_ends_takeover_history | PRD-17 | open | - |
| L-W6 | A: lessons_ws::one_byte_reads_decode_the_same | PRD-17 | open | - |
| L-W7 | A: lessons_ws::violations_are_named_and_relaying_goes_on | PRD-17 | open | - |
| L-W8 | A: proxy_ws.rs (PRD-03) | PRD-03 | open | - |
| L-W9 | A: lessons_ws::upgrade_101_and_first_frame_in_one_write | PRD-17 | open | - |
| L-W10 | A: lessons_ws::rejected_upgrade_stays_http; A: lessons_ws::bad_accept_is_a_named_loss_not_a_websocket | PRD-17 | open | - |
| L-W11 | A: PRD-03 rule 2 tests (file pending) | PRD-03 | open | - |
| L-W12 | A: lessons_ws::a_long_message_is_offered_before_its_last_frame | PRD-17 | open | - |
| L-W13 | A: lessons_ws::eof_mid_message_is_interrupted | PRD-17 | open | - |
| L-W14 | R: RR1 | PRD-12 | open | - |

## 2.6 Secrets (K). Guards live in PRD-05

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-K1 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K2 | A: mask_rules.rs (PRD-05) | PRD-05 | open | - |
| L-K3 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K4 | A: mask_corpus.rs (PRD-05); A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K5 | A: mask_rules.rs (PRD-05); R: RR1 (PRD-05 A2) | PRD-05 | open | - |
| L-K6 | A: mask_meta.rs (PRD-05); R: PRD-05 A3 | PRD-05 | open | - |
| L-K7 | A: mask_rules.rs (PRD-05) | PRD-05 | open | - |
| L-K8 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K9 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K10 | A: mask_corpus.rs (PRD-05) | PRD-05, PRD-11 | open | - |
| L-K11 | A: mask_rules.rs (PRD-05) | PRD-05, PRD-11 | open | - |
| L-K12 | A: mask_rules.rs (PRD-05); X: PRD-05 | PRD-05 | open | - |
| L-K13 | X: PRD-00, PRD-03 rule 5 | none | open | - |
| L-K14 | A: pipeline_secure.rs (PRD-02); R: PRD-12 C3 | PRD-02, PRD-12 | open | - |
| L-K15 | A: mask_stream.rs (PRD-05); A: lessons_ws::fail_closed_ends_at_message_end; R: RR1 | PRD-05, PRD-17 | open | - |
| L-K16 | A: mask_rules.rs (PRD-05); X: ledger M1 | PRD-05 | open | - |

## 2.7 Host names and addresses (N)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-N1 | A: lessons_hosts::authority_with_backslash_or_control_is_never_a_host | PRD-17 | open | - |
| L-N2 | A: lessons_hosts::unspecified_and_mapped_loopback_reach_this_computer | PRD-17 | open | - |
| L-N3 | A: lessons_hosts::loopback_names_and_numeric_labels_are_refused | PRD-17 | open | - |
| L-N4 | A: lessons_hosts::host_shape | PRD-17 | open | - |
| L-N5 | A: lessons_hosts::the_event_server_is_never_listed | PRD-17 | open | - |
| L-N6 | A: lessons_hosts::a_name_resolving_only_here_is_skipped | PRD-17 | open | - |
| L-N7 | A: connector_hints.rs (PRD-11) | PRD-11 | open | - |
| L-N8 | A: connector_launch.rs (PRD-11); A: connector_hints.rs (PRD-11) | PRD-11 | open | - |
| L-N9 | X: ledger H2, PRD-03 rule 2 | none | open | - |

## 2.8 Process, terminal, restart and environment (P)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-P1 | A: watchdog_restart.rs (PRD-02); R: RR4 | PRD-02 | open | - |
| L-P2 | A: proxy_restart.rs (PRD-03); A: watchdog_restart.rs (PRD-02); R: RR4 | PRD-02, PRD-03 | open | - |
| L-P3 | A: lessons_launch::ctrl_c_reaches_the_harness_and_the_watchdog_lives | PRD-17 | open | - |
| L-P4 | A: lessons_launch::a_tui_child_reads_the_terminal | PRD-17 | open | - |
| L-P5 | A: lessons_launch::modes_restored_after_a_raw_exit | PRD-17 | open | - |
| L-P6 | A: watchdog_rlimit.rs (PRD-02); R: PRD-02 R4; R: RR6 | PRD-02 | open | - |
| L-P7 | A: watchdog_end.rs (PRD-02) | PRD-02 | open | - |
| L-P8 | A: pipeline_seal.rs (PRD-02); R: PRD-11 acceptance 5 | PRD-02 | open | - |
| L-P9 | A: connector_launch.rs (PRD-11); R: PRD-12 gateway case | PRD-11 | open | - |
| L-P10 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-P11 | A: connector_launch.rs (PRD-11); R: PRD-10 acceptance 7 | PRD-11 | open | - |
| L-P12 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-P13 | R: RR6 (PRD-12 P9) | PRD-11 | open | - |
| L-P14 | A: cli_commands.rs (PRD-10) | PRD-10 | open | - |
| L-P15 | A: pipeline_seal.rs (PRD-02) | PRD-02 | open | - |

## 2.9 Session files (F). Guards live in PRD-04

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-F1 | A: native_walk.rs (PRD-04) | PRD-04 | open | - |
| L-F2 | A: native_tail.rs (PRD-04) | PRD-04 | open | - |
| L-F3 | A: native_safety.rs (PRD-04) | PRD-04 | open | - |
| L-F4 | A: native_tail.rs (PRD-04) | PRD-04 | open | - |
| L-F5 | A: native_snapshot.rs (PRD-04); A: native_safety.rs (PRD-04) | PRD-04 | open | - |
| L-F6 | A: native_snapshot.rs (PRD-04); A: native_room.rs (PRD-04) | PRD-04 | open | - |
| L-F7 | R: PRD-04 acceptance 2 | PRD-04 | open | - |
| L-F8 | X: PRD-11; R: PRD-12 P3 Hermes full row | PRD-11 | open | - |

## 2.10 Upload and server (S). Guards live in PRD-06 and PRD-07

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-S1 | A: server/worker/test/integration.test.ts (PRD-07, kept case at :292); A: upload_resume.rs (PRD-06) | PRD-06, PRD-07 | open | - |
| L-S2 | A: server/worker/test/integration.test.ts (PRD-07, kept case at :320) | PRD-07 | open | - |
| L-S3 | A: server/worker/test/integration.test.ts (PRD-07, kept case at :638) | PRD-07 | open | - |
| L-S4 | A: upload_beat.rs (PRD-06) | PRD-06 | open | - |
| L-S5 | A: upload_resume.rs (PRD-06) | PRD-06 | open | - |
| L-S6 | R: PRD-06 U3 | PRD-06 | open | - |
| L-S7 | A: upload_beat.rs::clock_set_back_does_not_lengthen_a_segment (PRD-06); A: cli_start.rs (PRD-10) | PRD-06, PRD-10 | open | - |
| L-S8 | R: PRD-06 U2; R: PRD-06 U5 | PRD-06 | open | - |
| L-S9 | R: PRD-06 U3 | PRD-06 | open | - |
| L-S10 | A: server/worker/test/ (PRD-07 case: a repeated open answers the same run; exact name pending) | PRD-07 | open | - |

## 2.11 Run 1's cause chain: never again (R)

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-R1 | A: pipeline_no_latch.rs (PRD-02) | PRD-02 | open | - |
| L-R2 | A: pipeline_guard.rs (PRD-02); A: pipeline_stall.rs (PRD-02) | PRD-02 | open | - |
| L-R3 | A: check_invariants.rs (PRD-08, invariant 12 gap_has_end); A: proxy_http1.rs (PRD-03, fault case); A: see L-H2, L-H9, L-W2, L-W7, L-W10 | PRD-08, PRD-03 | open | - |
| L-R4 | R: PRD-02 R3; R: PRD-10 acceptance 1 | PRD-02, PRD-10 | open | - |
| L-R5 | A: golden.rs (PRD-01); A: proxy_ws.rs (PRD-03) | PRD-01, PRD-03 | open | - |
| L-R6 | A: proxy_connect.rs (PRD-03) | PRD-03 | open | - |
| L-R7 | R: PRD-12 C1 | PRD-12 | open | - |
| L-R8 | A: mask_meta.rs (PRD-05); R: RR1 | PRD-05, PRD-09 | open | - |

## 2.12 Harness facts from DECISIONS and the measured runs (HF). Data, PRD-11

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-HF1 | R: PRD-11 acceptance 5 | PRD-11 | open | - |
| L-HF2 | D: connectors/hermes-cli.toml (the CA entry); R: RR6 | PRD-11 | open | - |
| L-HF3 | D: connectors/hermes-cli.toml plan.before, plan.after; A: watchdog_end.rs (PRD-02) | PRD-11, PRD-02 | open | - |
| L-HF4 | A: cli_commands.rs::second_start_on_one_home_refuses (PRD-10) | PRD-10 | open | - |
| L-HF5 | A: connector_launch.rs::codex_gets_no_dash_c_override (PRD-11); D: connectors/codex-cli.toml args | PRD-11 | open | - |
| L-HF6 | A: cli_continue.rs (PRD-10); D: docs/plans/2026-10-30-v1.0/PRD-10-cli-participant.md section 3.1 step 3 | PRD-10 | open | - |
| L-HF7 | R: PRD-12 pass rule | PRD-12 | open | - |
| L-HF8 | A: connector_hosts.rs (PRD-11) | PRD-11 | open | - |

## 2.13 Gateways and the sandbox (G). Measured on 28 Oct

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-G1 | A: see L-H11; R: PRD-12 P4 gateway rows | PRD-03, PRD-09 | open | - |
| L-G2 | D: connectors/model-hosts.toml (kind: gateway entries); R: PRD-12 P4 | PRD-11, PRD-12 | open | - |
| L-G3 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-G4 | R: PRD-12 gateway case config | PRD-12 | open | - |
| L-G5 | R: PRD-12 pad boxes (--non-persistent, tmux) | PRD-12 | open | - |

## 2.14 Format, writer and recovery (FW). Guards live in PRD-01, PRD-02 and PRD-08

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-FW1 | A: pipeline_guard.rs (PRD-02) | PRD-02 | open | - |
| L-FW2 | A: recovery.rs (PRD-01) | PRD-01 | open | - |
| L-FW3 | A: server_compare.rs (PRD-08); A: PRD-08 pruned local parts case (name pending) | PRD-08 | open | - |
| L-FW4 | A: check_invariants.rs (PRD-08) | PRD-08 | open | - |
| L-FW5 | A: recovery.rs (PRD-01) | PRD-01 | open | - |
| L-FW6 | A: pipeline_seal.rs (PRD-02); A: check_invariants.rs (PRD-08) | PRD-02, PRD-08 | open | - |
| L-FW7 | A: check_invariants.rs (PRD-08); A: recovery.rs (PRD-01) | PRD-01, PRD-08 | open | - |
| L-FW8 | A: recovery.rs (PRD-01); R: RR4 | PRD-01 | open | - |
| L-FW9 | A: server_compare.rs (PRD-08) | PRD-08 | open | - |

## 2.15 Verifier (VR). Guards live in PRD-08

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-VR1 | A: check_invariants.rs (PRD-08) | PRD-08 | open | - |
| L-VR2 | A: PRD-08 and PRD-09 panic-in-check case (name pending) | PRD-08, PRD-09 | open | - |
| L-VR3 | A: check_invariants.rs (PRD-08, name pending) | PRD-08 | open | - |
| L-VR4 | A: check_named.rs (PRD-08, name pending) | PRD-08 | open | - |

## 2.16 Start, continue and status (SC). Guards live in PRD-10

| ID | Guard | Owner | Status | Commit |
|---|---|---|---|---|
| L-SC1 | A: cli_folder.rs (PRD-10, name pending) | PRD-10 | open | - |
| L-SC2 | A: cli_folder.rs (PRD-10); X: PRD-07 (abandon route cut) | PRD-10, PRD-07 | open | - |
| L-SC3 | A: lessons_launch::nothing_is_written_to_the_terminal_while_the_harness_runs | PRD-17 | open | - |
| L-SC4 | A: cli_continue.rs (PRD-10); R: PRD-10 acceptance 10 | PRD-10 | open | - |
| L-SC5 | R: PRD-10 acceptance 6 | PRD-10 | open | - |
| L-SC6 | A: cli_finish.rs (PRD-10, name pending) | PRD-10 | open | - |
| L-SC7 | X: ledger L5, L6 (PRD-10 section 4) | PRD-10 | open | - |

## Real-run rows

Each rides on a pad run that already happens (PRD-12); the pass rule is in PRD-17 Acceptance.

| Row | Run | Lessons | Status |
|---|---|---|---|
| RR1 | Codex, direct key, pad prompt with subagents (PRD-12 P3) | L-W1, L-W4, L-W14, L-C7, L-K5, L-K15, L-R5, L-R8 | open |
| RR2 | Claude Code, direct key (PRD-12 P3) | L-D1, L-K6, L-K10 | open |
| RR3 | Hermes, direct key (PRD-12 P3) | L-T1, L-HF1 | open |
| RR4 | The kill test (PRD-01 acceptance 3, PRD-02 R1) | L-P1, L-P2 | open |
| RR5 | A core-set pad run that installs packages and uses git | L-T5, L-C6 | open |
| RR6 | The owner's Mac pad run (PRD-12 P9) | L-P6, L-P13, L-HF2 | open |
