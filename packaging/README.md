# Release staging

The draft v0.5 line is a planned source-only candidate pending owner approval. There is no
downloadable binary or installer, staged bundle, registry publication, signature, notarization or
qualification. The commands below describe private candidate tooling; they do not assert that an
artifact currently exists.

`scripts/package-release.py` assembles a directory for later native packaging work. The
result is explicitly **unsigned** and **unqualified**. This tool does not build binaries,
execute them, install services, create configuration or credentials, regenerate connector
pins, archive a bundle, sign it, upload it, or publish it.

Staging requires an explicit mode and six inputs, plus an optional seventh, `--profile`:

```sh
python3 scripts/package-release.py --stage \
  --repo /physical/path/to/goal-cup \
  --binaries /physical/path/to/prebuilt/bin \
  --output /physical/path/to/new/stage \
  --target aarch64-apple-darwin \
  --revision 0123456789abcdef \
  --version 0.5.0
```

Use canonical physical paths, such as the value produced by `pwd -P`. macOS's default
`$TMPDIR` and `/tmp` are themselves symlinks into `/private`, so a caller staging under
either must resolve first. Staging refuses a symlink or reparse point anywhere in an input or
output-parent path, including an ancestor. It also refuses output that is equal to, inside, or
an ancestor of either input. The output must not already exist.

The supported target triples are `x86_64-apple-darwin` and `aarch64-apple-darwin`. Linux is
best-effort until it is tested and Windows is planned (D42); neither has a bundle yet.

There is one profile, `m2-pilot`, and it is the default. The earlier `release` and
`foundation` profiles are gone: `release` required a `goalcup-publisher` binary no crate ever
defined, and `foundation` bundled the V1 path, which D48 deleted.

The staged layout:

```text
stage/
  bin/
    goalcup
    goalcup-daemon
    goalcup-verify
  connectors/
    codex-cli/...
    claude-code-cli/...
  bundle.json
```

Each staged connector entry is copied as a complete ordinary directory, including declared seeds,
and must have an ordinary schema-1
`harness.toml`. Its declared `id` must be a nonempty string exactly equal to the connector
directory basename. Mismatched or duplicate IDs, malformed TOML, links, reparse points, and
special files are refused. Connector
filenames, bytes, and ordinary permission modes are preserved; binaries receive mode `0755`,
and staged file modes are verified bit for bit.

`bundle.json` records format version 1, the exact target/revision/version arguments, the
`profile`, `qualification: "unqualified"`, `signature: "unsigned"`, the profile labels and the
connector profile hashes: the `profile_sha256` values are the exact staged `harness.toml`
bytes. Its sorted `files` array covers every payload file below `bin/` and
`connectors/`, recording a portable relative path, type `file`, SHA-256, byte size, and
four-digit octal intended mode. `bundle.json` does not inventory itself because a file cannot
contain its own stable cryptographic hash. Hashes are integrity metadata, not signatures. The
metadata contains no timestamps or absolute paths, so identical input bytes and modes produce
identical bundle content (`test_m2_pilot_staging_is_deterministic_across_two_runs` in
`scripts/test_package_release.py`).

The source tree and OSS cut contain exactly three pinned schema-1 directories: `codex-cli`,
`claude-code-cli` and `hermes-cli`. The private `M2_PILOT_CONNECTORS` allowlist selects only Codex
and Claude. It is a product choice, not a closure inferred from legacy manifests; a source pin
does not prove package inclusion, bundling, release, or qualification for any connector.

**The `m2-pilot` profile** stages the three binaries used for local M2 capture — `goalcup`,
`goalcup-daemon`, and `goalcup-verify` — with the pinned `codex-cli` and `claude-code-cli`
capture profiles. The proxy runs in-process under `goalcup start --goal-file`, so it is not a
separate binary. The bundle does not include `hermes-cli` (`M2_PILOT_CONNECTORS` in
`scripts/package-release.py`).
This profile is macOS-only and binds the exact source revision, target, version, binary and
connector-file hashes, plus each connector's exact schema `harness.toml` SHA-256 under the
compatibility key `profile_sha256`. Its metadata explicitly says
`m2-pilot`, `candidate`, `private`, `unsigned`, `unnotarized`, and `unqualified`. It excludes
harness installations or homes, keys, traces, reports, and caches. Installation
checks the candidate labels,
exact host target, source SHA, three-binary allowlist and both schema profile hashes before it
writes the disposable tracker prefix.

To stage manually, build the three binaries from the candidate source, then provide the exact
revision and target:

```sh
cargo build --release --offline -p goalcup-cli -p goalcup-daemon -p goalcup-verifier
python3 scripts/package-release.py --stage --profile m2-pilot \
  --repo "$(pwd -P)" --binaries "$(pwd -P)/target/release" \
  --output /physical/path/to/new/m2-pilot-stage \
  --target aarch64-apple-darwin --revision "$(git rev-parse HEAD)" \
  --version 0.0.0-m2-pilot-candidate
```

Use `x86_64-apple-darwin` for an Intel Mac. Install only into a new, disposable prefix while
evaluating the candidate:

```sh
sh packaging/macos/install-assisted.sh --profile m2-pilot \
  --bundle /physical/path/to/m2-pilot-stage \
  --prefix /physical/path/to/disposable/goalcup-prefix \
  --label com.agentcommunity.goalcup.m2pilot
```

The M2 daemon config is local-only: it omits `upload_url` and `server_public_key`, and no
receipt service is started. Use subscription authentication in each tracker's
isolated harness home. Do not set a harness model base URL or install a system CA. These steps
produce an unqualified private candidate; they do not establish harness qualification or a
public release. `sh packaging/macos/test-m2-pilot.sh` builds, stages, installs, checks a
controlled local v5 lifecycle and integrity result, then uninstalls from its own temporary
prefix. The synthetic workload deliberately lacks wire and native evidence, so its expected
integrity exit is 1; it is an installation smoke, not either real-harness journey.

> **Warning (evidence installs).** The installer bootstraps a persistent launchd user agent
> (`RunAtLoad`, so it comes back at every login; `KeepAlive` restarts it only after a crash, at
> most once per 10 seconds), and no run script boots it out: a daemon keeps running after the
> run. Stop it with `launchctl bootout gui/$(id -u)/<label>`, or remove it with
> `uninstall-assisted.sh`. `test-m2-pilot.sh` boots out only the disposable
> label it minted. `uninstall-assisted.sh` keeps
> `var/artifacts` and `var/workspaces` but deletes the install's `goalcup-verify` and daemon
> config, so verify or archive an entry's evidence before you uninstall.

Construction happens in a private temporary sibling. Files are opened read-only with
no-follow and nonblocking flags where the host provides them, checked as regular files by
the opened descriptor, and copied without applying metadata to the source. The tool compares
source identities, hashes, modes, and the connector tree before and after copying; detected
mutation aborts and removes only its own temporary directory. It validates copied hashes,
modes, and the complete payload file set before installing the final directory.

Final installation uses an atomic no-replace rename: `renamex_np(RENAME_EXCL)` on macOS,
`renameat2(RENAME_NOREPLACE)` on Linux, and Windows' non-replacing directory rename. A host
or filesystem without a safe primitive is refused explicitly. A destination created by a
racing process is preserved, and failed staging leaves no partial final directory.

This is an operator-side tool running with the operator's permissions. A same-user process
can still mutate paths between individual path checks and opens. Descriptor checks and the
two source snapshots detect many mutations and verify the copied payload against the snapshot
the tool observed. They do not pin the source or connector namespace: directory enumeration and
individual file opens are not one atomic snapshot. A replacement completed before the initial
snapshot can become that observed input, and a mutation after the final check can leave the live
source different from an already self-consistent staged bundle. Keep trusted inputs quiescent for
the entire bounded staging operation; native installer hardening and
release qualification are later work.

## Assisted macOS installation

`packaging/macos/` turns a staged `m2-pilot` bundle into a running, supervised
`goalcup-daemon` on this machine, without a package manager, a signing step, or root:

- `install-assisted.sh --bundle <staged-bundle> --prefix <fresh-dir> [--label <launchd-label>]`
  is the thin public entry point to `install_assisted.py`. The installer retains the bundle-root
  and destination-parent descriptors, refuses links and special nodes during a closed-tree walk,
  strictly validates canonical manifest paths and the launchd label, and re-verifies every staged
  file's hash, size and mode against `bundle.json` before publication. It creates
  `<prefix>/{bin,share/goalcup/connectors,etc,var/{artifacts,workspaces,log}}` at mode `0700`,
  copies the three binaries and the shipped connectors in, writes `<prefix>/etc/daemon.toml`
  (structurally reparsed after TOML escaping; loopback control address on an OS-assigned port,
  no upload service), structurally renders and reparses
  `com.agentcommunity.goalcup.plist` for that prefix and label, and loads it as a launchd
  **user** agent (`launchctl bootstrap gui/<uid>`) with `RunAtLoad` set and
  `KeepAlive = {Crashed = true}` and `ThrottleInterval = 10`. A crashed daemon is respawned, at
  most every ten seconds; one that exits cleanly, or refuses its configuration (exit 1), stays
  down. Nothing here touches a system LaunchDaemon or asks for a password. It refuses: a symlinked bundle or prefix path, any symlink in the prefix's
  ancestor chain, an ancestor not owned by this user or root (a shared sticky-bit directory
  such as `/tmp` is accepted), a named "broad root" (`/`, `$HOME`, `/Users`, `/System`, ...),
  a prefix that already exists, and an already-loaded plist at the chosen label.
- `uninstall-assisted.sh --prefix <installed-prefix> [--label <launchd-label>]` reads back
  `<prefix>/.goalcup-install-manifest.json` (refusing a prefix that has none, rather than
  guessing what to remove), unloads the launchd agent, removes the plist, `<prefix>/bin`,
  `<prefix>/share`, and the daemon config, and **always preserves** `<prefix>/var/artifacts`
  and `<prefix>/var/workspaces`: uninstalling is never how an entry's evidence is lost.
- `test-m2-pilot.sh` is the end-to-end smoke (see above).

**The profile has an explicit binary allowlist.** `bin/` is a flat directory keyed by binary
name in the staged bundle and `<prefix>/bin` after install.

Prefix and plist staging each use a fresh unpredictable mode-`0700` sibling namespace, with exact
directory modes independent of the caller's umask. The installer retains a descriptor for each
verified staged leaf. Its one authority-building traversal simultaneously compares the complete
staged prefix inventory with the inspected bundle manifest and the exact generated config/manifest
bytes: every relative name and node type, every exact directory/file mode, every regular-file size
and digest, with device/inode identities and directory sizes captured into the resulting immutable
snapshot. Registration therefore cannot turn a replacement introduced after earlier copying into a
new trusted baseline. The standalone plist is bound to its exact generated bytes, size, and mode in
the same step. After any test/coordination boundary and with control signals blocked, the installer
checks the retained leaf's exact mode and size, descriptor-relative name, and complete descendant
snapshot immediately before publication, then checks that the destination is that same retained
identity with the same exact mode and that its retained tree or file bytes remain unchanged
immediately after the syscall. Publication uses
descriptor-relative `renameatx_np(RENAME_EXCL)`, so a synchronized racing destination is preserved.
Rename success is latched before the post-check; an unexpected syscall or identity result is treated
as an ambiguous publication and reported for inspection rather than guessed away. When the safely
observed destination inode differs from the expected retained source inode, recovery reports both
identities explicitly.

Private-namespace cleanup is deliberately non-recursive and never automatically removes the outer
namespace: macOS has no inode-conditional `rmdir`, so even an empty, identity-checked namespace is
retained and reported rather than exposing a final check-to-remove race. Namespace open/finalization
failures likewise retain an authority when one exists or explicitly report the created path when it
could not be opened. Before the first publication, failures expose no final prefix, plist, or job,
but may retain a mode-`0700` private namespace for inspection. Successful installs also report their
two empty retained namespaces; disposable pilot teardown removes them only with its enclosing
temporary directory.

Before activation, retained descriptors must resolve to the original prefix and plist paths. The
installer opens and retains the daemon program and config beneath the retained prefix descriptor,
requires their descriptor-derived paths and identities to match the absolute paths embedded in the
structurally re-parsed plist, and rechecks the published prefix/plist identities and complete staged
tree, exact prefix-root mode `0700`, and exact plist mode `0644` immediately before bootstrap. A moved
or unavailable prefix or `LaunchAgents` parent, an observed descendant substitution, or a chmod-only
prefix/plist mutation therefore refuses activation and never bootstraps the stale or unverified
content. After either publication, failure never unlinks a
published object and never unloads the job: stderr records each path as the intended validation-time
path and, when available, its current descriptor-derived path, plus device/inode identity, label,
`gui/<uid>` domain, bootstrap/readiness state, and inspection-first manual recovery.

`SIGINT` and `SIGTERM` are converted to the same phase-aware refusal/recovery path. They are blocked
only across the process-global umask window and the exclusive-rename/immediate-latch window, then
handled with all descriptors still retained through the final report. `SIGKILL`, host power loss,
kernel failure, and abrupt process destruction cannot be caught; they can leave a private namespace,
one or both published objects, or a bootstrapped job at the last completed phase. A retry remains
exclusive and refuses those existing destinations, so recovery is inspection-first rather than an
automatic rollback. Bootstrap is marked attempted/possibly loaded before invocation, so interruption
during the command reports unknown state rather than “not attempted.” A same-UID process can still
mutate descendants between adjacent kernel operations, replace a published leaf, or replace the
same-label job after any observation; the pre/post publication and activation checks fail closed when
they observe such a change. Each complete-tree comparison is a sequence of descriptor-relative
opens and reads, not an indivisible filesystem snapshot; a same-UID process may still race between
those adjacent syscalls or mutate again after the final observation. Automatic cleanup cannot
distinguish a later replacement safely.

**What this proves, and what it does not.** This is the packaging path working end to end
against synthetic and self-built binaries on one Mac. It does **not** prove a real harness
session captures correctly through an installed bundle, does not prove behavior on Intel Macs
(only the host's own architecture is exercised), does not prove upgrade-over-existing-install
behavior (this script only ever installs into a fresh prefix), and makes no signing or
notarization claim anywhere. Notices required by a binary distribution are not yet collected;
generate them from the dependency set before the first binary release, as [NOTICE](../NOTICE)
requires.
