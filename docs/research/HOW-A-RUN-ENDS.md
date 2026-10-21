# How a run ends

Plain-English plan, 21 Oct 2026. Built from five research passes (agent decides it is done, a tool
the agent calls, GitHub, an API call, running out of time) and the owner's decisions.

## The short version

- **One clock.** The server starts the clock at the official start. The run ends 168 hours later. The
  participant's computer clock never decides anything.
- **At the end, nobody has to do anything.** Goal Cup Proxy stops the agent, seals the record and
  uploads it. If the computer was off, the server still ends the run on time.
- **What is judged is the last push GitHub received before the deadline**, on the repository's
  default branch. GitHub's own timestamp decides, not the commit date. Pushes after the deadline do
  not count and are flagged.
- **So "finishing" for an agent means: push your final work, then stop.** An agent that is done on
  day 2 just stops pushing. Its last push is its submission. No special button is needed.

## What the agent knows

- The goal-cup skill tells it: the run stops at a fixed deadline without warning, push often, and
  nobody can approve or answer anything, so decide everything yourself.
- It can ask how much time is left at any moment by running `goalcup time-left` in its shell. The
  answer comes from the server's clock. Goal Cup Proxy never pushes the time into the agent's context;
  the agent asks (D66).
- Today the skill reaches Codex and Claude Code. Hermes and Amp need a skills folder measured first,
  or the participant's prompt mentions `goalcup time-left`.

## What the participant does

- Nothing at the end. Leftover uploads go up by themselves the next time any `goalcup` command runs.
- During the run they may type only "keep going", for example after a quota reset.
- They may stop the agent early with `goalcup seal` from another terminal (it asks to confirm). That
  ends the current part; the run stays open until its deadline. It is rarely needed: an idle agent
  costs almost nothing to record.

## What gets recorded about the end

- The judged commit, its GitHub push time and branch, plus flags: pushed after the deadline,
  force-pushed during the run.
- The agent's last model call (when it really stopped working).
- How the stop went (a clean stop, or an agent process that could not be confirmed gone). That is
  never a penalty. Leftover processes only matter if they push, and late pushes are flagged anyway.

## Ideas we dropped, and why

- **A "finish" tool.** Models given one tend to quit early and throw away hours. It also adds a third
  way to stop a run.
- **An MCP server.** Every harness would need its own setup, none of it measured, two weeks before
  the event.
- **A submit API the agent calls.** The agent would need a key, and anything the agent can read, the
  human can read too.
- **A "final" tag or marker commit.** The agent can move or fake it, and the push timestamp already
  answers the question.

## Good ideas worth adding

1. **A "this is what will be judged" email.** Soon after the deadline, the server mails the participant
   the judged commit with a link. They can see at once what counts, and query it if it looks wrong.
   Server-only, small.
2. **Freeze the judged commit.** At the deadline the organizers copy the judged commit into their own
   storage, so a later force-push or a deleted repository cannot change what is judged. A small
   script, run by the organizers.
3. **"Hours worked" as a fact.** Report how many of the 168 hours the agent actually worked (first to
   last model call, minus idle time). Useful for research and fair to show next to results. Verifier
   report only.
4. **`GOALCUP.md` as the write-up.** Ask agents (through the participant's prompt or the skill) to keep a
   `GOALCUP.md` explaining the project. It is judged at the same final push as the code and can be the
   written material for the hackathon. Owner decision (21 Oct): required; the agent keeps it up to date (RULES, Submission).
5. **A clear end screen.** When the run ends, the terminal shows one short block: the deadline passed,
   the judged commit (once known), and "nothing else to do".

## Open decisions

- Hermes and Amp: measure a skills folder, or ask participants to mention `goalcup time-left` in their
  prompt?
- Ideas 1, 2 and 3: build before 4 Nov, or after the event?
