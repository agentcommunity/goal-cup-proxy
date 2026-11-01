# Security policy

How do I report a vulnerability, and what does this tool do on my machine? For anyone who runs
Goal Cup Proxy or reviews a change to it.

## Supported versions

Version 1.0 only. Release tarballs are unsigned and not notarized
([packaging/README.md](packaging/README.md)).

## Reporting a vulnerability

Do not open a public issue.

1. Email goalcup@agentcommunity.org.
2. Or use GitHub private vulnerability reporting: the Security tab, "Report a vulnerability".

Say what you ran, what you expected and what happened. Never attach a real trace, key or harness
transcript: they hold prompts and credentials. A synthetic reproduction is enough. We aim to
acknowledge a report within 7 days and will say what we plan to do. A small team maintains this
project. There is no bug bounty.

For an anti-cheat recorder, a way to bypass or forge capture is a vulnerability, not just a bug.
The gaps already accepted are under [What a check does not prove](#what-a-check-does-not-prove). A
report of one of those is known, not new, unless it shows that the stated bound does not hold.

## What runs on your machine

- **One recorder process and its watchdog.** `goalcup start` is the watchdog. It starts the
  recorder and the harness, restarts a recorder that dies, and ends the harness at the deadline
  and at the seal. There is no daemon and no service, nothing starts at login, and nothing is left
  running once the entry is sealed.
- **A loopback proxy.** The recorder listens on `127.0.0.1` only, on a port chosen for the entry.
  The harness reaches it through its proxy variables. Goal Cup never sets a model base URL.
- **A run CA, trusted only by the harness.** Each entry gets its own certificate authority. It is
  named only in the harness's own variables (`ca_env` in its connector), never added to the
  system trust store or a browser. Name constraints limit it to the hosts on the host list, and it
  expires after 8 days. Its key is a 0600 file in the run folder from the start of the entry to
  its seal, so `goalcup recover` can reuse the same CA after a crash and the harness keeps
  trusting it.
- **Isolated harness homes.** The harness runs with a home of its own, not your `~/.claude`,
  `~/.codex` or `~/.hermes`. Goal Cup writes nothing into those.
- **No keys held.** Goal Cup keeps no provider key and sets none. The harness's own credentials
  pass through the proxy's memory on the way to the model service and are masked before any byte
  of a record is written.
- **The upload.** The run credential is stored at mode 0600 under your Goal Cup folder and is
  never put in the harness's environment or argv. The recorder sends the record, in order, to
  `https://ingest.agentcommunity.org`. The seal also sends the list of connections passed through
  unread: host, port, times, bytes each way and the reason, never their content.

## How raw bytes stay off disk

- Every body, header and metadata string is masked before it is written. A masked byte becomes
  `*`, so lengths and offsets stay those of the source ([RECORD-FORMAT](docs/RECORD-FORMAT.md)).
- Core dumps are off for the recorder and the watchdog. A panic writes a fixed line with a file
  and line number and never its message, which could hold a recorded value.
- Credentials stay out of `goalcup.log`, argv, errors and `Debug` output.

The one place raw bytes can still land is memory paged out to swap. Nothing here locks memory, so
on Linux with unencrypted swap a page of the recorder can reach disk. macOS encrypts swap. If that
matters to you, use encrypted swap or none.

## Trust boundaries

| Who | Position |
|---|---|
| Other local accounts | Blocked by loopback and 0600/0700 files. |
| Web pages | The only listener is the loopback proxy. There is no control API to reach. |
| Same-user processes, including the agent under test | **Inside the boundary.** They can read the run folder and the run CA key while the entry is open. Detection is after the fact, from what the record shows. |
| Network | The proxy validates upstream TLS. Only the hosts on the host list are decrypted. Everything else, sign-in hosts included, tunnels unread. |
| The Goal Cup server | Holds the copy organizers trust. Nothing in a record is signed, so organizers rely on the server's copy and its `received_at` times, not on files on your machine. |
| Connector authors | **Trusted data.** See below. |

## Connectors are trusted data

A connector (`connectors/<id>.toml`) is compiled into the binary. It chooses the program, the
environment, the hosts decrypted and the masking rules. A change to one is read by a code owner
against the [reviewer checklist](CONTRIBUTING.md#reviewing-a-connector-pull-request). A file
connector (`--connector`, for practice and test runs only) is named in the record: the `start`
event holds `connector_source` `file` and the SHA-256 of the file, so judges can see it.

## What a check does not prove

`goalcup-verify check` says whether a record is complete and consistent. A pass is not a claim
that the participant followed the rules. Anything that behaves worse than stated here is a bug.

- **Masking has gaps.** It works from the shapes of known secrets and from values the recorder
  knows. A secret in a form nobody listed can survive. Masking keeps length, so the length of
  every masked value is visible.
- **Tunnels are unread.** A connection to a host not on the list, or a sign-in host, is passed
  through. The seal lists each with host, port, times and bytes. A model gateway behind such a
  host shows only as a named tunnel.
- **Vendor backends.** A harness can send a prompt to its vendor's own service. If that host is
  not a listed model host, the check cannot see the call.
- **The harness's own log is read only where a reader exists.** Elsewhere the check has the wire
  alone, and its coverage line says so.
- **The label is a hint.** `label` (`wire_and_native`, `wire_only`, `native_only`, `none`) names
  which sources of evidence a record has. Judges start from it. It is not a judgement.
- **Local files can change before upload.** The record is synced at most every 5 seconds and
  uploaded as it grows. A same-user process can rewrite the local files in that window. After
  upload, the server's copy is what counts.

## Out of scope

Root or same-user malware; a compromised harness binary; logging by the model provider; native
Windows (use WSL2).

## For contributors

Never commit `harnesses/`, `pad-out/`, real traces or keys, and read every fixture by eye. CI
runs no secrets on pull requests. See [CONTRIBUTING.md](CONTRIBUTING.md).
