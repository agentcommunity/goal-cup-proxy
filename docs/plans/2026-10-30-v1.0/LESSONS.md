# LESSONS: PRD-17 status table

The one map from each lesson in [PRD-17](PRD-17-lessons-old-code.md) sections 2.1 to 2.16 to its
guard. The case text of each row lives only in PRD-17 and is not copied here. Owned and kept by the
lessons lane (PRD-17); read by the deletion gate (PRD-17 Acceptance, checks 1 to 4).

- **Ids.** `L-<id>` here (`L-H2`); `PRD-17 <id>` everywhere else. The ledger and other PRDs reuse
  letters such as H2 with other meanings.
- **Guard codes.** `A:` an automated test, as `file::test` or as a test file name (Rust under
  `crates/*/tests/`, or a Worker test under `server/worker/test/`). `R:` a real-run check: a pad row (PRD-12) or RR1 to RR6 below. `D:` a data
  or docs file plus the key or section. `X:` the owner decision or PRD that dropped or inverted the
  case. Entries are separated by `; `, and no entry holds a `;` of its own. `see L-<id>` reuses that
  row's guard.
- **One name form.** A `file::test` entry is written without `.rs`
  (`recovery::a_sealed_entry_takes_nothing_more`); a file-level `A:` entry is a bare file name with
  its extension, never a path (`recovery.rs (PRD-01)`, `device.test.ts (PRD-07)`). The owner PRD of a test stands in parentheses after it,
  with at most a short qualifier; exact test titles go in the night log. `(name pending)` is the one
  pending marker: a case PRD-17 asked its owner to name in the night log.
- **Status.** `open` (not yet shown), `green` (every `A:` guard passes on integration in a build box),
  `real-run pass` (every `R:` guard shown in a pad run), `dropped: <decision>`. Every row starts `open`;
  rows whose only guard is `X:` stay `open` until the lane flips them at the gate. A row with both `A:`
  and `R:` guards reads `green` until its `R:` guards are shown, then `real-run pass`.
- **Commit.** The integration commit on which a build box proved every `A:` guard of the row, or
  `-`. It need not be the commit that changed the guarded code.
- **A renamed guard** is fixed by the lessons lane with an edit to this file and a night-log line. It
  is never a gate failure; only a lesson with no guard at all fails check 1.

## Lessons

`Sec` is the PRD-17 section (2.1 to 2.16) that holds the case text.

| ID | Sec | Guard | Owner | Status | Commit |
|---|---|---|---|---|---|
| L-C1 | 2.1 | A: lessons_connect::connect_and_client_hello_in_one_write_are_decrypted; A: lessons_connect::connect_head_split_over_one_byte_reads_is_decrypted | PRD-17 | open | - |
| L-C2 | 2.1 | A: lessons_connect::bad_heads_get_fixed_answers_without_echo | PRD-17 | open | - |
| L-C3 | 2.1 | A: lessons_connect::authority_forms | PRD-17 | open | - |
| L-C4 | 2.1 | A: lessons_connect::client_hello_in_two_records_and_one_byte_reads_decides_by_sni | PRD-17 | open | - |
| L-C5 | 2.1 | A: lessons_connect::close_before_hello_opens_no_upstream | PRD-17 | open | - |
| L-C6 | 2.1 | A: lessons_connect::unlisted_refusal_is_one_round; A: lessons_connect::listed_refusal_is_retried; A: proxy_connect.rs (PRD-03, the reset rule); R: RR5 | PRD-17 | open | - |
| L-C7 | 2.1 | R: RR1; A: lessons_connect::idle_tunnel_survives_120s (full tier) | PRD-17 | open | - |
| L-C8 | 2.1 | A: lessons_connect::one_bad_handshake_leaves_others_relaying | PRD-17 | open | - |
| L-C9 | 2.1 | A: proxy_connect.rs (PRD-03) | PRD-03 | open | - |
| L-C10 | 2.1 | A: proxy_connect.rs (PRD-03); A: lessons_connect::listed_sni_to_a_local_fake_server_is_never_trusted | PRD-03, PRD-17 | open | - |
| L-C11 | 2.1 | A: proxy_connect.rs (PRD-03); A: watchdog_rlimit.rs (PRD-02) | PRD-03 | open | - |
| L-C12 | 2.1 | A: watchdog_restart.rs (PRD-02) | PRD-02 | open | - |
| L-T1 | 2.2 | A: lessons_tls::leaf_aki_names_the_ca_key; R: RR3 | PRD-17 | open | - |
| L-T2 | 2.2 | A: lessons_tls::ca_constraints_validity_and_leaf_shape | PRD-17 | open | - |
| L-T3 | 2.2 | A: proxy_tls.rs (PRD-03); A: lessons_tls::untrusted_upstream_reaches_client_unchanged | PRD-03, PRD-17 | open | - |
| L-T4 | 2.2 | A: lessons_tls::ca_files_are_new_0600_and_never_follow_a_link (run-ca.pem and bundle.pem only; the `rec/run-ca.key` case is pending until PRD-02's writer is on integration) | PRD-17 | open | - |
| L-T5 | 2.2 | A: lessons_tls::bundle_holds_certificates_only_with_the_run_ca_first; A: lessons_tls::ca_files_are_new_0600_and_never_follow_a_link; R: RR5 | PRD-17 | open | - |
| L-T6 | 2.2 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-T7 | 2.2 | A: proxy_tls.rs (PRD-03, ALPN mirror matrix) | PRD-03 | open | - |
| L-T8 | 2.2 | A: proxy_tls.rs (PRD-03) | PRD-03 | open | - |
| L-T9 | 2.2 | D: docs/PARTICIPANT-GUIDE.md known limits (PRD-15) | PRD-03, PRD-15 | open | - |
| L-H1 | 2.3 | A: lessons_http1::head_scan_skips_blank_lines_and_takes_bare_lf | PRD-17 | open | - |
| L-H2 | 2.3 | A: lessons_http1::unframeable_heads_relay_on_inside_a_named_loss; A: proxy_http1.rs (PRD-03, generic fault case) | PRD-17 | open | - |
| L-H3 | 2.3 | A: lessons_http1::transfer_gzip_then_chunked_decodes | PRD-17 | open | - |
| L-H4 | 2.3 | A: lessons_http1::chunked_split_at_every_byte_ends_exactly; A: lessons_http1::chunk_size_rules; A: proxy_http1.rs (PRD-03, trailers) | PRD-17 | open | - |
| L-H5 | 2.3 | A: proxy_http1.rs (PRD-03); A: lessons_http1::framing_matrix | PRD-03, PRD-17 | open | - |
| L-H6 | 2.3 | A: proxy_http1.rs (PRD-03); A: lessons_http1::informational_then_final_keeps_the_pair | PRD-03, PRD-17 | open | - |
| L-H7 | 2.3 | A: proxy_http1.rs (PRD-03) | PRD-03 | open | - |
| L-H8 | 2.3 | A: lessons_http1::eof_inside_a_body_is_interrupted; A: lessons_http1::request_after_upstream_close_is_marked_unsent | PRD-17 | open | - |
| L-H9 | 2.3 | A: lessons_http1::head_over_64k_relays_inside_a_named_loss | PRD-17 | open | - |
| L-H10 | 2.3 | A: lessons_http1::authorization_forwarded_unchanged_and_starred_in_meta | PRD-17 | open | - |
| L-H11 | 2.3 | A: lessons_http1::every_request_on_a_decrypted_host_is_recorded | PRD-17 | open | - |
| L-H12 | 2.3 | X: PRD-03 rule 2 | PRD-03 | open | - |
| L-D1 | 2.4 | A: proxy_http1.rs (PRD-03); A: lessons_decode::brotli_whole_32k_pieces_and_trailing_bytes; R: RR2 | PRD-03, PRD-17 | open | - |
| L-D2 | 2.4 | A: lessons_decode::zstd_frame_ending_on_the_buffer_edge_is_complete; A: lessons_decode::zstd_cut_after_whole_pieces_is_a_gap; R: a pad run with a ChatGPT sign-in, else not shown in a real run | PRD-17 | open | - |
| L-D3 | 2.4 | A: lessons_decode::gzip_two_members_and_a_bad_crc | PRD-17 | open | - |
| L-D4 | 2.4 | A: lessons_decode::raw_and_zlib_deflate_split_after_byte_one | PRD-17 | open | - |
| L-D5 | 2.4 | A: proxy_http1.rs (PRD-03) | PRD-03 | open | - |
| L-D6 | 2.4 | A: lessons_decode::corrupt_tail_keeps_the_prefix (decoder layer only; the GAP and END records are shown once `proxy::serve` lands, see the note below the table) | PRD-17 | open | - |
| L-D7 | 2.4 | A: lessons_decode::bomb_relays_on_in_bounded_pieces | PRD-17 | open | - |
| L-D8 | 2.4 | A: lessons_decode.rs | PRD-17 | open | - |
| L-W1 | 2.5 | A: proxy_ws.rs (PRD-03); R: RR1 | PRD-03 | open | - |
| L-W2 | 2.5 | A: lessons_ws::negotiation_accepts_only_rfc7692; A: lessons_ws::unknown_extension_relays_inside_a_named_loss | PRD-17 | open | - |
| L-W3 | 2.5 | A: lessons_ws::server_max_window_bits_10_decodes | PRD-17 | open | - |
| L-W4 | 2.5 | A: lessons_ws::guard_drop_keeps_the_next_message_decodable; R: RR1 | PRD-17 | open | - |
| L-W5 | 2.5 | A: lessons_ws::bfinal_ends_takeover_history | PRD-17 | open | - |
| L-W6 | 2.5 | A: lessons_ws::one_byte_reads_decode_the_same | PRD-17 | open | - |
| L-W7 | 2.5 | A: lessons_ws::violations_are_named_and_relaying_goes_on | PRD-17 | open | - |
| L-W8 | 2.5 | A: proxy_ws.rs (PRD-03) | PRD-03 | open | - |
| L-W9 | 2.5 | A: lessons_ws::upgrade_101_and_first_frame_in_one_write | PRD-17 | open | - |
| L-W10 | 2.5 | A: lessons_ws::rejected_upgrade_stays_http; A: lessons_ws::bad_accept_is_a_named_loss_not_a_websocket | PRD-17 | open | - |
| L-W11 | 2.5 | A: lessons_ws::an_upgrade_on_any_path_is_recorded | PRD-17 | open | - |
| L-W12 | 2.5 | A: lessons_ws::a_long_message_is_offered_before_its_last_frame | PRD-17 | open | - |
| L-W13 | 2.5 | A: lessons_ws::eof_mid_message_is_interrupted | PRD-17 | open | - |
| L-W14 | 2.5 | R: RR1 | PRD-12 | open | - |
| L-K1 | 2.6 | A: mask_stream::every_split_equals_the_whole_record_masker (PRD-05) | PRD-05 | green | f5a200ae |
| L-K2 | 2.6 | A: mask_rules.rs (PRD-05) | PRD-05 | open | - |
| L-K3 | 2.6 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K4 | 2.6 | A: mask_corpus.rs (PRD-05); A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K5 | 2.6 | A: mask_rules::the_left_guard_keeps_escapes_and_drops_glue (PRD-05); A: mask_rules::a_base64url_blob_with_key_prefixes_inside_is_left_alone (PRD-05); R: RR1 (PRD-05 A2) | PRD-05 | green | f5a200ae |
| L-K6 | 2.6 | A: mask_meta.rs (PRD-05); R: PRD-05 A3; R: RR2 | PRD-05 | open | - |
| L-K7 | 2.6 | A: mask_rules.rs (PRD-05) | PRD-05 | open | - |
| L-K8 | 2.6 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K9 | 2.6 | A: mask_stream.rs (PRD-05) | PRD-05 | open | - |
| L-K10 | 2.6 | A: mask_corpus::the_billing_header_in_a_system_prompt_is_masked (PRD-05); R: RR2 | PRD-05, PRD-11 | green | f5a200ae |
| L-K11 | 2.6 | A: mask_rules.rs (PRD-05) | PRD-05, PRD-11 | open | - |
| L-K12 | 2.6 | A: mask_rules::url_password_cases (PRD-05); A: mask_rules::long_url_credentials_stay_masked (PRD-05); X: ledger M1 | PRD-05 | green | f5a200ae |
| L-K13 | 2.6 | X: PRD-00, PRD-03 rule 5 | none | open | - |
| L-K14 | 2.6 | A: pipeline_secure.rs (PRD-02); R: PRD-12 C3 | PRD-02, PRD-12 | open | - |
| L-K15 | 2.6 | A: mask_stream::a_fail_closed_window_ends_at_the_message (PRD-05); A: lessons_ws::fail_closed_ends_at_message_end; R: RR1 | PRD-05, PRD-17 | open | - |
| L-K16 | 2.6 | A: mask_stream::an_email_and_a_codex_rate_limit_object_stream_through (PRD-05); X: ledger M1 | PRD-05 | green | f5a200ae |
| L-N1 | 2.7 | A: lessons_hosts::authority_with_backslash_or_control_is_never_a_host | PRD-17 | open | - |
| L-N2 | 2.7 | A: lessons_hosts::unspecified_and_mapped_loopback_reach_this_computer | PRD-17 | open | - |
| L-N3 | 2.7 | A: lessons_hosts::loopback_names_and_numeric_labels_are_refused | PRD-17 | open | - |
| L-N4 | 2.7 | A: lessons_hosts::host_shape | PRD-17 | open | - |
| L-N5 | 2.7 | A: lessons_hosts::the_event_server_is_never_listed | PRD-17 | open | - |
| L-N6 | 2.7 | A: lessons_hosts::a_name_resolving_only_here_is_skipped | PRD-17 | open | - |
| L-N7 | 2.7 | A: connector_hints.rs (PRD-11) | PRD-11 | open | - |
| L-N8 | 2.7 | A: connector_launch.rs (PRD-11); A: connector_hints.rs (PRD-11) | PRD-11 | open | - |
| L-N9 | 2.7 | X: ledger H2, PRD-03 rule 2 | none | open | - |
| L-P1 | 2.8 | A: watchdog_restart.rs (PRD-02); R: RR4 | PRD-02 | open | - |
| L-P2 | 2.8 | A: proxy_restart.rs (PRD-03); A: watchdog_restart.rs (PRD-02); R: RR4 | PRD-02, PRD-03 | open | - |
| L-P3 | 2.8 | A: lessons_launch::ctrl_c_reaches_the_harness_and_the_watchdog_lives | PRD-17 | open | - |
| L-P4 | 2.8 | A: lessons_launch::a_tui_child_reads_the_terminal | PRD-17 | open | - |
| L-P5 | 2.8 | A: lessons_launch::modes_restored_after_a_raw_exit | PRD-17 | open | - |
| L-P6 | 2.8 | A: watchdog_rlimit.rs (PRD-02); R: PRD-02 R4; R: RR6 | PRD-02 | open | - |
| L-P7 | 2.8 | A: watchdog_end.rs (PRD-02) | PRD-02 | open | - |
| L-P8 | 2.8 | A: pipeline_seal.rs (PRD-02); R: PRD-11 acceptance 5 | PRD-02 | open | - |
| L-P9 | 2.8 | A: connector_launch.rs (PRD-11); R: PRD-12 gateway case | PRD-11 | open | - |
| L-P10 | 2.8 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-P11 | 2.8 | A: connector_launch::with_home_swapped_a_login_shell_finds_goalcup (PRD-11); R: PRD-10 acceptance 7 | PRD-11 | green | f5a200ae |
| L-P12 | 2.8 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-P13 | 2.8 | R: RR6 (PRD-12 P9) | PRD-11 | open | - |
| L-P14 | 2.8 | A: cli_commands.rs (PRD-10) | PRD-10 | open | - |
| L-P15 | 2.8 | A: pipeline_seal.rs (PRD-02) | PRD-02 | open | - |
| L-F1 | 2.9 | A: native_walk.rs (PRD-04) | PRD-04 | open | - |
| L-F2 | 2.9 | A: native_tail.rs (PRD-04) | PRD-04 | open | - |
| L-F3 | 2.9 | A: native_safety.rs (PRD-04) | PRD-04 | open | - |
| L-F4 | 2.9 | A: native_tail.rs (PRD-04) | PRD-04 | open | - |
| L-F5 | 2.9 | A: native_snapshot.rs (PRD-04); A: native_safety.rs (PRD-04) | PRD-04 | open | - |
| L-F6 | 2.9 | A: native_snapshot.rs (PRD-04); A: native_room.rs (PRD-04) | PRD-04 | open | - |
| L-F7 | 2.9 | R: PRD-04 acceptance 2 | PRD-04 | open | - |
| L-F8 | 2.9 | X: PRD-11; R: PRD-12 P3 Hermes full row | PRD-11 | open | - |
| L-S1 | 2.10 | A: integration.test.ts (PRD-07); A: upload_resume.rs (PRD-06) | PRD-06, PRD-07 | open | - |
| L-S2 | 2.10 | A: integration.test.ts (PRD-07) | PRD-07 | green | b474329d |
| L-S3 | 2.10 | A: integration.test.ts (PRD-07) | PRD-07 | green | b474329d |
| L-S4 | 2.10 | A: upload_beat.rs (PRD-06) | PRD-06 | open | - |
| L-S5 | 2.10 | A: upload_resume.rs (PRD-06) | PRD-06 | open | - |
| L-S6 | 2.10 | R: PRD-06 U3 | PRD-06 | open | - |
| L-S7 | 2.10 | A: upload_beat::clock_set_back_does_not_lengthen_a_segment (PRD-06); A: cli_start.rs (PRD-10) | PRD-06, PRD-10 | open | - |
| L-S8 | 2.10 | R: PRD-06 U2; R: PRD-06 U5 | PRD-06 | open | - |
| L-S9 | 2.10 | R: PRD-06 U3 | PRD-06 | open | - |
| L-S10 | 2.10 | A: device.test.ts (PRD-07); A: lifecycle.test.ts (PRD-07) | PRD-07 | green | b474329d |
| L-R1 | 2.11 | A: pipeline_no_latch.rs (PRD-02) | PRD-02 | open | - |
| L-R2 | 2.11 | A: pipeline_guard.rs (PRD-02); A: pipeline_stall.rs (PRD-02) | PRD-02 | open | - |
| L-R3 | 2.11 | A: check_invariants.rs (PRD-08, invariant 12 gap_has_end); A: proxy_http1.rs (PRD-03, fault case); A: see L-H2, L-H9, L-W2, L-W7, L-W10 | PRD-08, PRD-03 | open | - |
| L-R4 | 2.11 | R: PRD-02 R3; R: PRD-10 acceptance 1 | PRD-02, PRD-10 | open | - |
| L-R5 | 2.11 | A: golden.rs (PRD-01); A: proxy_ws.rs (PRD-03); R: RR1 | PRD-01, PRD-03 | open | - |
| L-R6 | 2.11 | A: proxy_connect.rs (PRD-03) | PRD-03 | open | - |
| L-R7 | 2.11 | R: PRD-12 C1 | PRD-12 | open | - |
| L-R8 | 2.11 | A: mask_meta.rs (PRD-05); R: RR1 | PRD-05, PRD-09 | open | - |
| L-HF1 | 2.12 | R: PRD-11 acceptance 5; R: RR3 | PRD-11 | open | - |
| L-HF2 | 2.12 | R: RR6 | PRD-11 | open | - |
| L-HF3 | 2.12 | D: connectors/hermes-cli.toml before, after; A: watchdog_end.rs (PRD-02) | PRD-11, PRD-02 | open | - |
| L-HF4 | 2.12 | A: cli_commands::second_start_on_one_home_refuses (PRD-10) | PRD-10 | open | - |
| L-HF5 | 2.12 | A: connector_launch::codex_gets_no_dash_c_override (PRD-11); D: connectors/codex-cli.toml args | PRD-11 | green | f5a200ae |
| L-HF6 | 2.12 | A: cli_continue.rs (PRD-10); D: docs/plans/2026-10-30-v1.0/PRD-10-cli-participant.md section 3.1 step 3 | PRD-10 | open | - |
| L-HF7 | 2.12 | R: PRD-12 pass rule | PRD-12 | open | - |
| L-HF8 | 2.12 | A: connector_hosts.rs (PRD-11) | PRD-11 | open | - |
| L-G1 | 2.13 | A: see L-H11; R: PRD-12 P4 gateway rows | PRD-17, PRD-09 | open | - |
| L-G2 | 2.13 | D: connectors/model-hosts.toml (kind: gateway entries); R: PRD-12 P4 | PRD-11, PRD-12 | open | - |
| L-G3 | 2.13 | A: connector_launch.rs (PRD-11) | PRD-11 | open | - |
| L-G4 | 2.13 | R: PRD-12 gateway case config | PRD-12 | open | - |
| L-G5 | 2.13 | R: PRD-12 pad boxes (--non-persistent, tmux) | PRD-12 | open | - |
| L-FW1 | 2.14 | A: pipeline_guard.rs (PRD-02) | PRD-02 | open | - |
| L-FW2 | 2.14 | A: recovery::a_part_file_cut_at_any_byte_recovers_to_its_last_index_line (PRD-01) | PRD-01 | green | f5b0a6a2 |
| L-FW3 | 2.14 | A: server_compare::missing_middle_chunk_fails (PRD-08); A: server_compare::fw3_pruned_entry_is_named_and_server_copy_stands_in (PRD-08) | PRD-08 | green | f5a200ae |
| L-FW4 | 2.14 | A: check_invariants.rs (PRD-08) | PRD-08 | open | - |
| L-FW5 | 2.14 | A: recovery::a_part_file_cut_at_any_byte_recovers_to_its_last_index_line (PRD-01); A: recovery::a_boot_with_no_index_line_is_deleted_and_an_r_closed_part_file_takes_a_second_frame (PRD-01); A: recovery::a_sealed_entry_takes_nothing_more (PRD-01) | PRD-01 | green | f5b0a6a2 |
| L-FW6 | 2.14 | A: pipeline_seal.rs (PRD-02); A: check_invariants.rs (PRD-08) | PRD-02, PRD-08 | open | - |
| L-FW7 | 2.14 | A: check_invariants.rs (PRD-08); A: recovery.rs (PRD-01) | PRD-01, PRD-08 | open | - |
| L-FW8 | 2.14 | A: recovery::a_part_file_cut_at_any_byte_recovers_to_its_last_index_line (PRD-01); R: RR4 | PRD-01 | green | f5b0a6a2 |
| L-FW9 | 2.14 | A: server_compare::fw9_one_flipped_byte_fails_the_copy (PRD-08) | PRD-08 | green | f5a200ae |
| L-VR1 | 2.15 | A: check_invariants.rs (PRD-08) | PRD-08 | open | - |
| L-VR2 | 2.15 | A: usage_run1.rs (PRD-09, a panic inside `usage` prints file and line, never record text, name pending); A: server_compare::vr2_seal_without_reason_or_totals_is_reported (PRD-08, exit below 3 only, not a panic test); A: check_invariants::vr3_every_mutation_fails_its_check_with_exit_2 (PRD-08, the inv5 row, exit code only); X: PRD-08 Amendments, slice 2 (the panic hook in `main.rs`, file and line then exit 3, stays a review check because no test input reaches it) | PRD-08, PRD-09 | open | - |
| L-VR3 | 2.15 | A: check_invariants::vr3_every_mutation_fails_its_check_with_exit_2 (PRD-08) | PRD-08 | green | f5a200ae |
| L-VR4 | 2.15 | A: check_named::clean_record_with_interrupted_end_exits_0 (PRD-08) | PRD-08 | green | f5a200ae |
| L-SC1 | 2.16 | A: cli_folder::project_folder_and_goalcup_home_never_hold_each_other (PRD-10) | PRD-10 | green | f5a200ae |
| L-SC2 | 2.16 | A: cli_folder::refused_start_creates_nothing (PRD-10, folder half; the other cases wait on approve.rs); X: PRD-07 (abandon route cut) | PRD-10, PRD-07 | open | - |
| L-SC3 | 2.16 | A: lessons_launch::nothing_is_written_to_the_terminal_while_the_harness_runs | PRD-17 | open | - |
| L-SC4 | 2.16 | A: cli_continue.rs (PRD-10); R: PRD-10 acceptance 10 | PRD-10 | open | - |
| L-SC5 | 2.16 | R: PRD-10 acceptance 6 | PRD-10 | open | - |
| L-SC6 | 2.16 | A: cli_finish::unsealed_status_never_reads_as_finished (PRD-10) | PRD-10 | green | f5a200ae |
| L-SC7 | 2.16 | X: ledger L5, L6 (PRD-10 section 4) | PRD-10 | open | - |

**Pending record halves.** `lessons_decode.rs` (D1 to D4, D6) drives `proxy::decode` alone, so it
shows the decoded prefix and the `Step`, not the GAP `decode_failed` and END `decode_error` records.
L-D1 to L-D4 and L-D6 are not flipped until the record half runs through `proxy::serve` with the
fake client (L-D8), in `lessons_http1.rs` or by extending those tests. L-T4 likewise waits for the
`rec/run-ca.key` case. L-VR2 waits for PRD-09's named `usage` case.

## Real-run rows

Each rides on a pad run that already happens (PRD-12). The run and the pass rule of each row are in
PRD-17 Acceptance; which lessons a row shows is read from the `R:` entries above, not listed here.

| Row | Status |
|---|---|
| RR1 | open |
| RR2 | open |
| RR3 | open |
| RR4 | open |
| RR5 | open |
| RR6 | open |
