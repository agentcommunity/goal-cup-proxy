# Goal Cup registration, teams, and tracer authorization

**Date:** 2026-10-05
**Status:** Agreed conversational design written for owner review. Written-spec approval and an implementation plan are still required before implementation.
**Milestone:** V1.1 event infrastructure. This document does not change the qualified V1 foundation claim or the active ten-step execution schedule.

## 1. Intent and scope

Give an individual or an ad hoc team one Goal Cup home on `https://agentcommunity.org/goal-cup`. A participant can register early, connect a tracer through a familiar CLI-to-browser approval journey, test the complete system, and start independent competition entries during the official global starting window.

Reuse the website's community identity and membership infrastructure. All people accessing a team dashboard or authorizing a tracer must be signed-in community members and confirmed members of that team. The CLI connects a tracer installation that manages measured agent runs; the human operator handles questionnaires, videos, sharing, and final submission on the website.

This first sub-project covers event registration, membership and team permissions, acceptance records, browser continuation, installation authorization, and the ownership contract consumed by future run services. It does not implement production evidence ingestion, storage, processing, live metrics, ranking, or event programming. Those require a separate spec and implementation plan. Practice recording depends on that later ingestion service; auth alone cannot deliver an end-to-end practice run.

## 2. V1 reference and M2 integration target

**Implementation-target review:** local `m2/wire` at `99352ed4`, worktree `/Users/team/dev/worktrees/goal-cup/m2-wire`, inspected 2026-10-05. The product decisions in this spec stand; the CLI/service plan must target the released successor of this M2 line rather than assume V1's active runtime. The observations below are a snapshot, not a claim that the branch is released or production-ready.

- M2's default product records native plus wire evidence in v5 and is LocalOnly: no production server, receipt, or acknowledgement. `goalcup auth`, V1 `resume`, and the old upload/runtime path were deleted with D48 (and `v1-compat` with them); the old auth code in git history is a reuse candidate.
- V5 entries currently cannot resume. Section 8 describes desired event behavior and requires separately designed and qualified M2 continuity support; turning on V1 compatibility does not supply it.
- The v5 record format is frozen by D42/D43 and `docs/CONTRACTS.md`. Optional `Launch.run_public_key` and `Launch.connector_dir_sha256` already exist. Add event/identity/start and delivery claims through versioned server records or side artifacts where possible; do not add account/team/mode fields to frozen chain records without the format's explicit evolution decision.
- Preserve the independent LocalOnly recorder. Community signup is required for Goal Cup participation and authenticated server testing, not for all standalone local recording. LocalOnly is neither automatically a registered practice entry nor a competition entry.
- The server-phase brief proposes daemon-owned packing/signing and ACK persistence, with CLI-owned TLS and bearer credentials. It proposes clockless delivery registration first, competition receipts later, and post-seal upload before live streaming. These are local planning proposals, not approved implementation in this spec. A clockless registration or a post-seal ACK cannot establish an official competition start or live continuity.
- Local planning inputs, **local, not in git**: `.superpowers/sdd/2026-10-02-oss/post-lane-a-plan.md` and `server-phase-brief.md` in the M2 worktree. The lane plan has active file ownership/refactor boundaries; defer CLI integration until its affected seams settle. Do not dispatch or alter those lanes from this auth task. Reconcile the older brief with current source and D43 before using its signatures or feature assumptions.
- D42 plans a fresh public allowlist copy rather than publishing this internal history. This event design stays internal unless explicitly selected for publication. An implementation plan must pin the eventual released source and verify which reuse modules it actually contains.

The following V1 foundations remain useful reference, with their behavior distinguished from M2:

- A V1 `goalcup auth` command once implemented browser-opening device authorization, bounded polling, `/v0/device` and `/v0/token`, and secret grant storage at mode `0600` outside the measured workspace. It was deleted with D48 (`crates/goalcup-cli/src/commands/` has start, status, entries, seal and doctor); git history keeps it as a starting point.
- `agentCommunity_PAGE/features/auth/AUTH.md` describes Supabase browser authentication using Google, GitHub, and email OTP. Human authentication and existing general agent credentials are separate issuers/flows. Goal Cup installation grants must not silently reuse general agent scopes or change that product's authorization contract.
- Existing rules allow multiple parallel entries, require verified community accounts, and establish a global 24-hour starting window with a separate 168-hour clock per entry. Optional community endorsement is not a participation requirement.
- Production organizer authentication, TLS ingestion, durable acknowledgements, private signing keys, and the eventual 168-hour qualification are V1.1 work. Existing development signing and loopback upload are not production proof.
- `goalcup-protocol` owns exchanged and persisted byte definitions; the daemon records raw evidence and health, and the verifier interprets evidence. Preserve those boundaries.
- Never set a harness model base URL or write app-wide harness configuration. Harness authentication remains separate from Goal Cup authorization.
- Every cargo command requiring dependency resolution is offline; run `sh scripts/gate.sh fast` before committing. Connector changes require regenerated pins. No raw spike output, harness installations, or build artifacts belong in commits.

Relevant authorities: `TODO.md`, `docs/RULES.md`, `docs/DECISIONS.md` and `docs/TERMINATION.md` (the older `STATUS.md`, boundary spec and ten-steps plan are in git history only). This is a proposed additive event design; discrepancies below require explicit reconciliation, not silent rewriting of historical contracts.

## 3. Official registration and participation funnel

Luma is discovery and sharing only. Its aggregate interest number may be reported separately; no Luma registration, identity matching, or integration is required for participation.

Website registration is authoritative and available before the starting window. CLI authorization can create the same event registration if the person has not registered on the website. Registration is unique per participant and event, and repeated sign-in or authorization must not create duplicates.

Participant progress is derived from records, not one overloaded account-status column:

| Stage | Evidence |
|---|---|
| Registered | Event registration completed |
| Tracer connected | An active installation authorization exists |
| Started | At least one competition entry has an authoritative start |
| Submitted | At least one competition entry has a completed submission packet |

Practice activity is shown separately and does not satisfy competition-started or competition-submitted stages. A participant can have running, abandoned, and submitted entries simultaneously. Capture finish, packet completion, eligibility, compliance review, and judging are separate concepts.

## 4. Accounts, individuals, and teams

An account uses the existing individual or organization signup. Goal Cup participation is either individual or team; organization affiliation is optional. An ad hoc team does not need to register as an organization.

An individual participant has one operator. A team has one captain initially and can contain multiple operators. The captain must already be a community member when creating the team. A person using an existing organization account follows its existing authority rules; an organization email is not permission to share credentials or proof of every human operator's identity.

The captain can add teammates by name and email. An invitation is a pending roster record, not an account, membership confirmation, terms acceptance, or dashboard permission. A teammate signs in or joins through their own account and explicitly accepts the invitation. Binding uses the verified identity and valid invitation, not an arbitrary email supplied by the client. Joining the community does not automatically accept every team invitation addressed to that person.

| Actor | Proposed first-version permission |
|---|---|
| Individual operator | Own dashboard, tracer authorizations, entries, and final submission |
| Team captain | Team dashboard; roster management; own tracer authorizations and runs; team final submission |
| Confirmed team member | Team dashboard; own tracer authorizations; start, resume, and finish their own managed entries |
| Pending invitee | Complete signup and invitation confirmation; no team dashboard or tracer authorization |

The member can see shared team entry status and presentation material; dashboard access is not blanket permission to download raw evidence or other installations' secrets. Captain authority over another operator's running process is not assumed. Captain transfer, account deletion during a live run, and cross-operator recovery need an explicit follow-up policy before those operations are exposed.

A team can start with one confirmed member while invitations remain pending. Anyone operating a tracer must confirm membership before doing so. Every teammate included in the final roster must join and confirm before final submission; this deadline is submission, not the end of agent execution. Pending invitations must be resolved or excluded from the submitted roster. A later captain addition must not retroactively confer authority over an earlier run. Recording roster changes provides an audit trail.

## 5. Browser home and acceptance records

Goal Cup has its own dashboard at `/goal-cup`; it reuses community identity without placing the competition inside `/members`. A members-area link can point to it. Every Goal Cup signup and authorization continuation returns to the Goal Cup surface, including after provider redirects and email verification.

| Entry point | Journey |
|---|---|
| Early website registration | Sign in/join; complete event onboarding; accept competition terms and acknowledge the privacy notice; land on Goal Cup dashboard |
| CLI authorization | Open Goal Cup approval page; sign in/join; complete missing event onboarding and acceptances; choose an eligible individual/team participant; explicitly approve installation; land on Goal Cup dashboard |

Reuse current recorded acceptances instead of asking again. Record the person, event, document type, version/content identifier, and server acceptance time. Updated competition terms can require renewed acceptance before a new start. Privacy-notice changes need an appropriate acknowledgment policy; do not treat all notice acknowledgment as consent to every processing purpose. A captain cannot accept documents or marketing permission for teammates.

The browser displays capture scope, evidence access, intended publication, and the installation's permissions before approval. Competition communications use the registration contact and preferences; optional community/service/promotional campaigns require separately recorded permissions and suppression/unsubscribe handling. Entering a teammate's email permits an invitation workflow, not enrollment in promotions.

Continuations use validated internal destinations and an opaque authorization reference. Preserve the continuation across authentication while preventing open redirects, secret leakage into URLs, and approval through link visits alone. A signed-in browser session does not by itself approve a machine.

## 6. Tracer authorization journey

1. The participant runs `goalcup auth` (proposed; no such command ships today).
2. The CLI asks them to press Enter, then opens the system browser on the official Goal Cup approval surface. It prints a fallback verification URL and user code for another-browser/headless use.
3. The browser authenticates the person, completes missing registration/acceptances, and shows the installation request with eligible participant choices. Team requests require confirmed membership.
4. The person verifies the matching code and explicitly approves the installation for the chosen participant.
5. The CLI polls the service, receives its grant, and stores the secret outside the agent-readable workspace. No token is printed or logged.
6. The CLI confirms the connected participant. The browser remains on the Goal Cup dashboard.

Use the existing device-flow client as the starting point. A future complete verification URL may prefill the user code, but must preserve human confirmation and must not contain the secret device credential. Approval attempts expire, are consumed atomically, and have bounded polling and guessing protection. Denial, expiry, cancellation, failed browser opening, and changed membership have explicit outcomes. Revalidate membership and acceptances at approval, not only when the page first renders.

The grant identifies the installation, authorizing person, selected participant/team, event, audience, permissions, and validity. One account can authorize multiple installations; another terminal on the same installation can reuse its grant. Reauthorization must not reassign an existing entry or reset its clock. Changing participant context requires a new explicit authorization binding rather than silently mutating existing ownership.

Authorization grants only the ability to request eligible starts, upload evidence for the installation's own bound entries, and resume/finish its managed runs. It cannot edit community details, marketing preferences, roster membership, or another installation's runs. Entry-scoped upload authority is derived and verified server-side; the later ingestion spec determines its credential format.

The dashboard lists installations, who authorized each, authorization time, last contact, and managed entries. Server-side expiry/revocation and current membership checks govern future requests. Removing membership revokes future team permissions while preserving historical ownership/evidence; the eventual service must define how interrupted uploads are recovered without granting removed members continuing access. Revocation is not proof of process termination, especially offline. Preserve local evidence and show last confirmed state accurately.

Credential expiry, rotation/renewal, and recovery must support an uninterrupted seven-day run without interactive mid-run login under normal operation. Exact lifetimes and recovery behavior are follow-up design inputs, not invented constants in this spec.

## 7. Practice and official starts

Authorization is available before the event and launches no agent, starts no recording, and creates no measured entry by itself. Early participants can run readiness checks and explicitly request practice.

| Mode | Start policy | Data path | Competition effect |
|---|---|---|---|
| Practice | Explicit practice request; available before the event | Full capture, upload, processing, dashboard, recovery, finish, and submission test | Excluded from competition rankings and prizes |
| Competition | Server accepts a new start only within the official global 24-hour window | Same evidence path with competition designation | Eligible for later review, not automatically a passing submission |

Proposed participant commands are `goalcup start --practice` and ordinary `goalcup start`; existing harness selection remains required. Practice defaults to 10 minutes, allows early finish, and permits a requested longer duration under a server-approved policy. No silent fallback from competition to practice. Practice duration policy and production CLI/protocol changes belong to the later run-services plan.

Ordinary start outside the window explains the official opening/closing time and offers practice. The official client checks the schedule before launching a competition harness. The server independently enforces the window at the authoritative start transition; setup or arming inside the window does not reserve a later start. A refusal at that boundary must close the unstarted attempt safely and report that no competition entry started. Client clock edits, earlier authorization, and pending network retries cannot manufacture eligibility. An official start requires server approval; outage behavior must not silently convert it to LocalOnly. This does not prevent an explicitly chosen independent local recording.

Run mode, participant, operator, installation, event, start, and deadline are server-bound and immutable once issued. Design an explicit competition-start authorization compatible with M2's frozen evidence format and versioned side artifacts; legacy receipts and clockless delivery registrations cannot implicitly qualify as production competition starts. Idempotent start handling returns the same decision for the same request without creating duplicate entries. The official start signal must be specified for M2's `--goal-file` headless launch rather than assumed to be V1's first typed prompt; the required outcome is server-authorized start within the window before autonomous goal execution. Resolve this launch/receipt ordering in the run-service spec before official mode ships.

Registered practice uses a distinct workspace, session, prompt, and run identity. It is never promoted to competition, and its product output cannot seed a competition workspace. A real entry starts fresh. Practice retention and organizer access need a published policy before server-connected practice capture is offered. LocalOnly tests remain local and do not upload merely because the installation has signed in; participating in server-connected practice must be explicit.

This design cannot stop a modified open-source client from recording privately. It prevents that recording from receiving an accepted official competition start outside the window.

## 8. Parallel entries, interruption, and replacement

One individual or team can own multiple entries, including parallel entries on multiple authorized installations. Each has a separate root session, prompt, workspace, evidence, and 168-hour clock.

- **Resume (event requirement, unavailable for current M2 v5):** continues the same bound session, prompt, workspace, entry, and deadline. The clock never pauses. The subsequent M2 continuity design must preserve this contract and explicitly reconcile competition assistance rules; do not reuse V1 commands as proof of v5 continuity.
- **Start another:** creates an independent fresh entry while other entries continue.
- **Abandon and replace:** closes the old entry with its recorded history retained, then creates a fresh entry during the starting window. Replacement gets its own 168 hours from its accepted start; a changed prompt always creates a new entry.

Abandonment is a product designation and requires safe finish/finalization semantics, not an assertion that every process has stopped. The eventual implementation must preserve the termination contract in `docs/TERMINATION.md`. A link to the replaced entry can explain the relationship without merging their evidence.

After the window closes, existing runs can resume against original deadlines; new competition starts and replacements are refused. Signing in on a new installation does not transfer a live run. Cross-machine recovery remains outside this first design.

## 9. Database and later ingestion boundary

Use the existing Supabase identity and community records. Add event-specific records under independently enforced server authorization and row-level access controls. The following are conceptual entities, not migration/table names:

| Entity | Responsibility |
|---|---|
| Event | Official schedule and policy/document versions |
| Participant | Individual or team ownership and optional organization affiliation |
| Membership/invitation | Captain/member roles, verified confirmation, pending invitations, change history |
| Registration | Unique participant/event enrollment and operator contact |
| Acceptance | Per-person, per-event document/version acknowledgment or acceptance |
| Installation authorization | Person/participant/event binding, protected credential state, expiry/revocation |
| Run, later phase | Mode, immutable ownership/start binding, lifecycle, health and evidence references |
| Submission, later phase | Human presentation packet and completion state separate from run finalization/verdict |

Store names/emails and roster permissions privately. Store installation credentials as protected secret verifiers or appropriate cryptographic material, not plaintext browser-readable rows. Roster, acceptance, credential, and registration writes must be atomic where they jointly grant authority. Enforce permissions on the server and through database controls, not just hidden buttons.

The future ingestion path is: authorized installation and bound entry → authenticated evidence upload → private object storage with durable references → verifier/metric processing → permitted summary rows → dashboard. Every request checks the bound entry; caller-provided account/team identifiers are not ownership evidence. Large traces/artifacts belong in private object storage, not dashboard database rows.

Signed acknowledgements, retry deduplication, crash recovery, redaction, source binding, private signing keys, retention, and processing reconciliation require their own service spec. Preserve the daemon's record-raw boundary and run-key custody. Assess the proposed CLI transport/daemon signing-and-ACK seam against current M2 contracts; do not copy V1's daemon network path into the capture product. Practice and competition use the same processing interfaces, with mode enforced throughout aggregation and ranking. Database acceptance or receipt of an upload is not a compliance verdict. Post-seal delivery can establish receipt of sealed evidence, but the agreed full streaming practice test requires live delivery; expose any earlier post-seal milestone with an accurate readiness label.

## 10. Dashboard and operator work

First auth/registration release: participant/team selection, registration state, acceptance flow, invitations, installation approvals/connections, official schedule, and setup/support links. Early registration can include event questions and optional social sharing; additional personal questions require a defined purpose and exact form specification before implementation.

Later run/dashboard release: entries with prompt, harness/model, start/deadline, last contact, lifecycle, health, and presentation checklist. Metrics include token usage and observed subagent counts where supported; show unavailable values and freshness rather than invented zeroes. Raw transcript billing/credit information is not a public metric. Public sharing of prompts/results follows announced publication policy, not automatic publication at auth time.

Operators provide before/after reports through browser forms. Accept accessible public or unlisted YouTube links without login/password gates. Before video is encouraged early, with a soft target within the first 24 hours of that entry. No penalty was agreed. Final video and questionnaire accompany final submission; current rules describe a 24-hour presentation period after the run. A validated link at submission does not guarantee permanent availability.

Rankings, judging/feedback, setup sessions, programming during the week, prize/winner details, and wider promotional campaigns are later product work. Social-sharing bonuses and any change to existing scoring require an explicit rule decision; optional sharing must not become a hidden signup requirement.

## 11. Acceptance scenarios for the subsequent plan

1. Early website signup creates one registration, records the person's acceptances, and lands on Goal Cup. Repeating it creates no duplicate.
2. A CLI-first participant completes signup in the browser and returns to Goal Cup, including OAuth and cross-device email verification; unrelated membership routes retain their existing destinations.
3. Current acceptances are reused, changed required terms block new starts pending browser acceptance, and a captain's acceptance never covers another person.
4. A pending invitation grants no dashboard/tracer access; the verified invitee confirms it. A different user cannot claim it. A member removed during approval cannot receive a grant.
5. A member can authorize multiple installations for an explicitly selected participant. Tokens do not enter URLs, logs, browser projections, or measured workspaces; deny/expire/replay attempts cannot mint a valid grant.
6. Team members can see the team dashboard but cannot mutate another installation's entries; final submission belongs to the captain and requires a resolved, confirmed submitted roster.
7. Auth alone launches/captures nothing. Explicit 10-minute practice tests the full future path and never enters competition aggregates or promotes to competition.
8. Competition start is refused before opening and after closing, including an attempt armed before closing but first-prompted afterward. Server time is authoritative and repeated requests cannot duplicate entries.
9. Parallel entries retain separate prompts/workspaces/clocks. Resume keeps the deadline; replacement needs a fresh accepted start in the window. A new login resets none of these.
10. Uploads for another entry/team are refused. Dashboard summaries expose only permitted data; revocation and offline state never imply confirmed remote process termination.

## 12. Follow-up decisions and review boundary

The agreed product behavior above is sufficient for owner review, but these inputs must be settled in the appropriate plan/spec before dependent code is built:

- Auth implementation plan: exact provider continuation integration, device endpoint compatibility/versioning, document versions/copy, installation credential lifetime/renewal, invitation mechanics, and schema/RLS/RPC names. Captain transfer/deletion can remain unavailable until specified.
- Run-service spec: production host/storage/key ownership, receipt mode/ownership encoding, window boundary/retry policy, practice-duration limits, evidence retention/access, membership-removal recovery, and start/abandon lifecycle changes.
- Operator/submission spec: exact onboarding questions, roster cutoff enforcement, video/questionnaire fields, public projections, and any scoring effects.

Known documentation reconciliation: `docs/RULES.md` currently says website acceptance occurs before a device code is shown, while the existing CLI creates the code before browser approval. The agreed new flow requires acceptance before the installation grant is issued, allowing a CLI-first browser signup. Reconcile that wording with the client/server contract in the auth implementation change. Also document event registration separately from automatic tracked-entry creation, team/operator authority, practice exclusion, and explicit replacements. Do not rewrite the current rules while this spec is still awaiting review.

Review this written spec first. Once approved, use `superpowers:writing-plans` for the auth/registration sub-project only; produce separate plans for later services instead of one oversized implementation plan. No implementation or deployment is authorized by writing this document.

## References

- [OAuth device authorization, RFC 8628](https://www.rfc-editor.org/rfc/rfc8628.html): browser verification, bounded device polling, complete verification URI, and confirmation considerations.
- [OAuth for native apps, RFC 8252](https://www.rfc-editor.org/rfc/rfc8252.html): external browser and PKCE/loopback alternative considered during brainstorming.
- [Supabase Auth documentation](https://supabase.com/docs/guides/auth): existing identity foundation; not a claim that production Goal Cup endpoints already exist.
- [ICO direct marketing planning guidance](https://ico.org.uk/for-organisations/direct-marketing-and-privacy-and-electronic-communications/direct-marketing-guidance/plan-direct-marketing/): separate optional promotional permission from participation and service communications. Applicable jurisdiction and final legal copy require their own review.
