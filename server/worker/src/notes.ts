// Server notes: the rows of advisories.json that match a run's connector, harness version and OS,
// sent as `notes` in the open and continue answers (docs/UPLOAD-API.md). goalcup prints each one.

export interface Advisory {
  /** Each is the exact value, or "*" for any. `version` also matches as a numeric prefix ("0.22" holds 0.22.6). */
  connector: string;
  version: string;
  os: string;
  note: string;
}

export const NOTES_MAX = 5;
export const NOTE_MAX_CHARS = 200;
const ROWS_MAX = 50;

/** The numbers of the first `\d+(\.\d+)*` in `text` ("Hermes Agent v0.22.6" gives [0, 22, 6]); none: []. */
const numbers = (text: string): number[] => text.match(/\d+(\.\d+)*/)?.[0].split(".").map(Number) ?? [];

/** `spec`'s numbers are a prefix of the version's: "0.22.6" holds 0.22.6, not 0.22.60; "0.22" holds 0.22.6, not 0.220. */
function versionMatches(spec: string, harnessVersion: string): boolean {
  if (spec === "*") return true;
  const want = numbers(spec);
  const have = numbers(harnessVersion);
  return want.length > 0 && want.length <= have.length && want.every((n, i) => n === have[i]);
}

const holds = (spec: string, value: string) => spec === "*" || spec === value;

/** The notes for these facts, in file order, at most five. */
export function pickNotes(rows: readonly Advisory[], facts: { connector: string; harnessVersion: string; os: string }): string[] {
  return rows
    .filter((r) => holds(r.connector, facts.connector) && holds(r.os, facts.os) && versionMatches(r.version, facts.harnessVersion))
    .slice(0, NOTES_MAX)
    .map((r) => r.note);
}

/** `{ notes }` when there are any, else nothing, so an answer without notes is unchanged. */
export const notesField = (notes: string[]) => (notes.length === 0 ? {} : { notes });

/** Null when `rows` is a valid advisories file, else the first problem. */
export function validAdvisories(rows: unknown): string | null {
  if (!Array.isArray(rows)) return "not an array";
  if (rows.length > ROWS_MAX) return `more than ${ROWS_MAX} rows`;
  for (const [i, row] of rows.entries()) {
    if (typeof row !== "object" || row === null || Array.isArray(row)) return `row ${i} is not an object`;
    const keys = Object.keys(row).sort().join(",");
    if (keys !== "connector,note,os,version") return `row ${i} must have exactly connector, version, os and note`;
    if (!Object.values(row).every((v) => typeof v === "string")) return `row ${i} has a value that is not a string`;
    const { version, note } = row as Advisory;
    if (!/^(\*|\d+(\.\d+)*)$/.test(version)) return `row ${i}: version must be "*" or numbers and dots`;
    if (note.length < 1 || note.length > NOTE_MAX_CHARS) return `row ${i}: note must be 1-${NOTE_MAX_CHARS} characters`;
    if (/[\p{Cc}‪-‮⁦-⁩]/u.test(note)) return `row ${i}: note has a control or bidi character`;
  }
  return null;
}
