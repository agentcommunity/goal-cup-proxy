// Box time accounting for the conductor. Pure helpers; box.mjs does the API calls.

function round1(n) { return Math.round(n * 10) / 10; }

// Sum box minutes and vCPU minutes over sessions. A running session counts up to nowMs.
export function sessionMinutes(sessions, nowMs) {
  let minutes = 0;
  let vcpuMinutes = 0;
  for (const s of sessions) {
    const start = s.startedAt ?? s.createdAt;
    const end = s.stoppedAt ?? nowMs;
    const m = Math.max(0, end - start) / 60000;
    minutes += m;
    vcpuMinutes += m * s.vcpus;
  }
  return { minutes: round1(minutes), vcpuMinutes: round1(vcpuMinutes) };
}

// One ledger line: HH:MMZ | conductor spend | ...
export function ledgerLine(t, now) {
  const hhmm = now.toISOString().slice(11, 16);
  let line = `${hhmm}Z | conductor spend | ${t.boxes} boxes since ${t.since}: ${t.minutes} box minutes, ${t.vcpuMinutes} vCPU minutes`;
  if (t.modelUsd !== undefined) line += `; model about ${t.modelUsd} USD billed`;
  return line;
}
