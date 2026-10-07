# Docs standard

How do I write or change a doc here? For anyone editing Markdown in this repository.
Updated: 2026-10-02.

## One question per doc

The first line under the title states that question and who the doc is for. If a paragraph
answers a different question, link to the doc that owns that question.

## Maximum lengths

- Entry and state docs (AGENTS, STATUS): 120 lines.
- Guides (README, onboarding, contributing, cheat checklist, rules): 200.
- ARCHITECTURE: 250. CONNECTOR-CONTRIBUTING (procedure plus reviewer checklist): 240.
  Reference (OPERATING): 450. Index files: 100.
- A ticket line: 4 lines while open, 1 or 2 when done. A line is at most 100 characters.
- Exempt: DECISIONS entries, CONTRACTS.md, the invariant block in TERMINATION, qualification
  record bodies, spike reports, format specs.

## Pointers, not copies

- Use relative Markdown links, for example
  `[CONTRACTS C6](CONTRACTS.md)`.
- Use `path:line` only for code, and only when a reader must land on one spot. Prefer a symbol
  name (`FINISH_COMMIT_RESERVE`); line numbers rot.
- A number (gate counts, bytes, durations, SHAs) appears in exactly one current place: LEDGER
  for tickets, the qualification record for measurements. Every other doc links there.
  Re-derive a number from its source before writing it; never copy it from another summary.
- Never cite a git-ignored path (`.superpowers/`, `harnesses/`, `spikes/*/out/`) as evidence
  without the words "local, not in git".

## Current versus history

- A current doc has no "earlier", "historical", "at that checkpoint" or "superseded"
  paragraphs. When something becomes history, move it to `docs/archive/` or to its
  qualification record and leave a one-line link.
- A historical file that stays in place gets exactly one banner as the first line after the
  title: `> **Historical (YYYY-MM-DD).** <one clause why>. Current: [link].`
- No stacked banners. Replace the old banner instead of adding one on top.
- Each doc has one `Updated: YYYY-MM-DD` line. Do not keep a dated changelog inside it.
- Write "on `m2/wire`" or "on `v1/trunk`" only where behaviour differs. The default subject of
  these docs is `m2/wire`.

## Words

- Use the [glossary](GLOSSARY.md) terms exactly. Do not coin synonyms: not "trace file" for
  chain, "hook leg" for wire leg, or "finalize" for seal.
- Write "measured" only with a linked record. Write "built" for code that exists. Write
  "qualified" only for T27/T28-level acceptance.
- When a fact is unknown, write UNKNOWN. Do not guess.
