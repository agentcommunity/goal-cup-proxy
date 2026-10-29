# Vision

What we are building, and the rules every change is judged against. Read this before planning or
building anything. When older docs or decisions disagree with it, raise it with the maintainer.

## Goal Cup in one page

Goal Cup is a hackathon where the agent builds. Edition 01 is run by
[Agent Community](https://agentcommunity.org). The event site is
[agentcommunity.org/goalcup](https://agentcommunity.org/goalcup). Some details there may be slightly
out of date; the full rules in [docs/RULES.md](docs/RULES.md) win.

- **One prompt, one agent, up to 168 hours.** A participant prepares harness, model and tools, writes
  one goal, and steps back. After the start, people may only send "keep going".
- **One root agent in one harness per entry.** It may start its own subagents, as many as it likes.
- **Start window:** 7 November 2026, 00:00 UTC, to 8 November 2026, 00:00 UTC. Every entry gets its
  own 168 hours from its start.
- **A fresh product from scratch.** The repository is public from the start and ends under the MIT
  licence.
- **Two short videos:** one on the goal and setup by hour 24, one demo after the run.
- **Free, online, bring your own harness and your own key.** Solo, as a team or as a company.
- **Judged by humans.** The judges read the prompt and setup, not only the product.

Goal Cup Proxy records each run. The record shows the rules held, and the runs are the research.

## The seven points

1. **Record every model call** the harness makes, to any provider, for the whole run.
2. **Record every subagent,** however many and however long.
3. **Never block or slow the harness.** If something cannot be recorded, note a gap and keep going.
4. **Never record secrets.**
5. **Upload everything,** so the server can rebuild the full story by ID and time.
6. **Tell the participant plainly** what was recorded.
7. **Work with the harnesses people actually use,** run the way they use them.

## How we build

- **Less code.** Fewer places for things to break. Cut before you add.
- **No caps of our own** on subagents, calls or bytes. If a limit is needed, it guards a real cost,
  and it says so.
- **No backward compatibility before Edition 01.** Every run so far is test data. The protocol,
  verifier, Worker and client may change together.
- **Ready by Friday 6 November 2026,** the day before the start window opens.
- **Test it the way people will use it.** Ask for subagents in the prompt; never force them by config.
