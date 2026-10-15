//! The connector **directory** pins this build ships, as data (harness contract v1, S2).
//!
//! `pins/connector-dirs.txt` holds one line per connector folder, `<id> <sha256>`. It sits
//! outside `connectors/`, so it never hashes itself. This module reads it at **build time**:
//! `include_str!` brings the bytes in and a `const fn` parser turns them into the table. A
//! malformed, unsorted, duplicated or uppercase line panics during const evaluation, so the
//! crate (and the signed daemon that embeds the table) does not compile. Nothing is parsed
//! at run time and no `build.rs` is involved.
//!
//! # Format
//!
//! * Optional leading `#` comment lines, then the pin lines. A `#` after the first pin line
//!   is refused.
//! * One pin per line: `<id> <sha256>\n`, `id` matching `^[a-z][a-z0-9-]{1,47}$`, the digest
//!   64 lowercase hex characters, a single space between them, nothing after the digest.
//! * Sorted by id (byte order), ids unique, a trailing newline, LF line endings.
//! * At least one pin: an empty table would pin nothing.
//!
//! # Why a plain-function twin
//!
//! A panic in const evaluation can only carry a literal, so the build error cannot name the
//! bad line. [`parse_pins_text`] runs the same scanner at run time and reports the line
//! number and the reason; the unit tests exercise every refusal through it, and
//! `tests/it/connector_pins.rs` runs it over the committed file.
//!
//! A pin is a tamper check after the build, not a defence against a malicious merge:
//! review is the control for that (`docs/DECISIONS.md` D43, ruling 5).

use crate::ProtocolError;
use std::ffi::OsStr;

/// The bytes of `pins/connector-dirs.txt`.
const DATA: &str = include_str!("../pins/connector-dirs.txt");

/// Why a pin file is refused. `Copy` and literal-only so the const parser can turn each
/// into a distinct `panic!` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinProblem {
    /// The file holds no pin line.
    NoPins,
    /// The last byte is not `\n`.
    NoTrailingNewline,
    /// A blank line.
    BlankLine,
    /// A carriage return anywhere: the file is LF only.
    CarriageReturn,
    /// A `#` line after a pin line.
    CommentAfterPins,
    /// The id is shorter than 2 or longer than 48 bytes, or is not `[a-z][a-z0-9-]*`.
    BadId,
    /// No single space after the id, or more than one.
    BadSeparator,
    /// The digest is not exactly 64 lowercase hex characters.
    BadDigest,
    /// This id sorts before the one above it.
    NotSorted,
    /// This id repeats the one above it.
    Duplicate,
}

impl PinProblem {
    /// The sentence a refusal carries at run time.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            PinProblem::NoPins => "holds no pin line, so it would pin nothing",
            PinProblem::NoTrailingNewline => "does not end with a newline",
            PinProblem::BlankLine => "is a blank line; the file is comments, then pins",
            PinProblem::CarriageReturn => "holds a carriage return; the file is LF only",
            PinProblem::CommentAfterPins => "is a comment after the first pin line",
            PinProblem::BadId => "has an id that is not `^[a-z][a-z0-9-]{1,47}$`",
            PinProblem::BadSeparator => "needs exactly one space between the id and the digest",
            PinProblem::BadDigest => "has a digest that is not 64 lowercase hex characters",
            PinProblem::NotSorted => "is not in id order (ids ascend by byte)",
            PinProblem::Duplicate => "repeats the id of the line above it",
        }
    }

    /// The build failure. One literal per problem, because const evaluation cannot format.
    const fn fail(self) -> ! {
        match self {
            PinProblem::NoPins => {
                panic!("pins/connector-dirs.txt holds no pin line, so it would pin nothing")
            }
            PinProblem::NoTrailingNewline => {
                panic!("pins/connector-dirs.txt does not end with a newline")
            }
            PinProblem::BlankLine => {
                panic!("pins/connector-dirs.txt holds a blank line; it is comments, then pins")
            }
            PinProblem::CarriageReturn => {
                panic!("pins/connector-dirs.txt holds a carriage return; it is LF only")
            }
            PinProblem::CommentAfterPins => {
                panic!("pins/connector-dirs.txt holds a comment after the first pin line")
            }
            PinProblem::BadId => {
                panic!(
                    "pins/connector-dirs.txt holds an id that is not [a-z][a-z0-9-], 2 to 48 bytes"
                )
            }
            PinProblem::BadSeparator => panic!(
                "pins/connector-dirs.txt needs exactly one space between an id and its digest"
            ),
            PinProblem::BadDigest => panic!(
                "pins/connector-dirs.txt holds a digest that is not 64 lowercase hex characters"
            ),
            PinProblem::NotSorted => {
                panic!("pins/connector-dirs.txt is not sorted by id: run scripts/pin-manifests.sh")
            }
            PinProblem::Duplicate => {
                panic!("pins/connector-dirs.txt repeats an id: run scripts/pin-manifests.sh")
            }
        }
    }
}

/// A refusal of a pin file: the 1-based line and the problem. A whole-file problem
/// (`NoPins`, `NoTrailingNewline`) names the last line, or line 0 for an empty file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinsError {
    pub line: usize,
    pub problem: PinProblem,
}

impl core::fmt::Display for PinsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "pins/connector-dirs.txt line {} {}",
            self.line,
            self.problem.describe()
        )
    }
}

impl std::error::Error for PinsError {}

/// Where one pin sits in the file: byte offsets, so the const parser can slice it.
#[derive(Clone, Copy)]
struct Row {
    id_start: usize,
    id_len: usize,
    sha_start: usize,
}

const ZERO_ROW: Row = Row {
    id_start: 0,
    id_len: 0,
    sha_start: 0,
};

const ID_MIN: usize = 2;
const ID_MAX: usize = 48;
const SHA_LEN: usize = 64;

/// Byte order comparison of `data[a..a+alen]` with `data[b..b+blen]`: -1, 0 or 1.
const fn cmp_ranges(data: &[u8], a: usize, alen: usize, b: usize, blen: usize) -> i8 {
    let mut i = 0;
    while i < alen && i < blen {
        let (x, y) = (data[a + i], data[b + i]);
        if x < y {
            return -1;
        }
        if x > y {
            return 1;
        }
        i += 1;
    }
    if alen < blen {
        -1
    } else if alen > blen {
        1
    } else {
        0
    }
}

/// Scan the whole file. Validates every line; records each pin's position in `out` when
/// `store` is set (`out` must then hold at least as many rows as there are pins). Returns the
/// pin count. This one function is both the build-time parser and the run-time twin.
#[allow(clippy::manual_range_contains)] // `RangeInclusive::contains` is not `const`.
const fn scan(data: &[u8], out: &mut [Row], store: bool) -> Result<usize, PinsError> {
    let mut pos = 0;
    let mut line_no = 0;
    let mut count = 0;
    let mut prev = ZERO_ROW;
    while pos < data.len() {
        line_no += 1;
        // The line is data[pos..end]; `end` is the index of its `\n`, or data.len().
        let mut end = pos;
        while end < data.len() && data[end] != b'\n' {
            end += 1;
        }
        let len = end - pos;
        // A carriage return anywhere in the line.
        let mut k = pos;
        while k < end {
            if data[k] == b'\r' {
                return Err(PinsError {
                    line: line_no,
                    problem: PinProblem::CarriageReturn,
                });
            }
            k += 1;
        }
        if end == data.len() {
            return Err(PinsError {
                line: line_no,
                problem: PinProblem::NoTrailingNewline,
            });
        }
        if len == 0 {
            return Err(PinsError {
                line: line_no,
                problem: PinProblem::BlankLine,
            });
        }
        if data[pos] == b'#' {
            if count > 0 {
                return Err(PinsError {
                    line: line_no,
                    problem: PinProblem::CommentAfterPins,
                });
            }
            pos = end + 1;
            continue;
        }
        // `<id> <sha>`: the id runs to the first space.
        let mut sp = pos;
        while sp < end && data[sp] != b' ' {
            sp += 1;
        }
        let id_len = sp - pos;
        let id_ok = id_len >= ID_MIN && id_len <= ID_MAX && {
            let mut ok = data[pos] >= b'a' && data[pos] <= b'z';
            let mut j = pos + 1;
            while j < sp {
                let b = data[j];
                ok = ok && ((b >= b'a' && b <= b'z') || (b >= b'0' && b <= b'9') || b == b'-');
                j += 1;
            }
            ok
        };
        if !id_ok {
            return Err(PinsError {
                line: line_no,
                problem: PinProblem::BadId,
            });
        }
        // Exactly one space, then the digest to the end of the line.
        if sp >= end || sp + 1 >= end || data[sp + 1] == b' ' {
            return Err(PinsError {
                line: line_no,
                problem: PinProblem::BadSeparator,
            });
        }
        let sha_start = sp + 1;
        let mut sha_ok = end - sha_start == SHA_LEN;
        let mut j = sha_start;
        while sha_ok && j < end {
            let b = data[j];
            sha_ok = (b >= b'0' && b <= b'9') || (b >= b'a' && b <= b'f');
            j += 1;
        }
        if !sha_ok {
            return Err(PinsError {
                line: line_no,
                problem: PinProblem::BadDigest,
            });
        }
        if count > 0 {
            match cmp_ranges(data, prev.id_start, prev.id_len, pos, id_len) {
                0 => {
                    return Err(PinsError {
                        line: line_no,
                        problem: PinProblem::Duplicate,
                    });
                }
                1 => {
                    return Err(PinsError {
                        line: line_no,
                        problem: PinProblem::NotSorted,
                    });
                }
                _ => {}
            }
        }
        let row = Row {
            id_start: pos,
            id_len,
            sha_start,
        };
        if store {
            out[count] = row;
        }
        prev = row;
        count += 1;
        pos = end + 1;
    }
    if count == 0 {
        return Err(PinsError {
            line: line_no,
            problem: PinProblem::NoPins,
        });
    }
    Ok(count)
}

/// The text of `data[start..start+len]`. The bytes were validated as ASCII by [`scan`].
const fn text(data: &'static str, start: usize, len: usize) -> &'static str {
    let (_, tail) = data.as_bytes().split_at(start);
    let (head, _) = tail.split_at(len);
    match core::str::from_utf8(head) {
        Ok(s) => s,
        Err(_) => panic!("pins/connector-dirs.txt: a validated field is not UTF-8"),
    }
}

/// The number of pins in `data`, or a build failure.
pub(crate) const fn count_pins(data: &'static str) -> usize {
    let mut none: [Row; 0] = [];
    match scan(data.as_bytes(), &mut none, false) {
        Ok(n) => n,
        Err(e) => e.problem.fail(),
    }
}

/// The pins in `data`, or a build failure. `N` is [`count_pins`] of the same text.
pub(crate) const fn parse_pins<const N: usize>(
    data: &'static str,
) -> [(&'static str, &'static str); N] {
    let mut rows = [ZERO_ROW; N];
    match scan(data.as_bytes(), &mut rows, true) {
        Ok(_) => {}
        Err(e) => e.problem.fail(),
    }
    let mut out = [("", ""); N];
    let mut i = 0;
    while i < N {
        let r = rows[i];
        out[i] = (
            text(data, r.id_start, r.id_len),
            text(data, r.sha_start, SHA_LEN),
        );
        i += 1;
    }
    out
}

const N: usize = count_pins(DATA);
const PINS: [(&str, &str); N] = parse_pins::<N>(DATA);

/// The connector **directory** digests this build ships, `(connector_id, sha256)`: the
/// release allowlist of manifests, seeds and shims.
///
/// A connector is a directory: its manifest(s), and the files under `files/` that the
/// tracker renders or copies into a run. A pin over one manifest file alone would read as
/// "this is the shipped connector" for a directory whose genuine manifest sits beside an
/// edited seed. The digest covers every byte in the directory
/// ([`crate::connector_dir::connector_dir_sha256`] carries the exact layout).
///
/// Read from `pins/connector-dirs.txt` at build time (see the module docs) and regenerated
/// by `scripts/pin-manifests.sh`, never typed by hand. It lives in the protocol crate
/// because the daemon reads it and must never depend on the verifier (D3): a pinned digest
/// is data, and the daemon's use of it stays a comparison against a constant.
pub const CONNECTOR_DIR_SHA256S: &[(&str, &str)] = &PINS;

/// `Some(pinned_sha)` when this build pins the connector's whole directory, `None` when it
/// does not: [`CONNECTOR_DIR_SHA256S`] read by id.
///
/// By the id the manifest DECLARES: a connector id is never joined onto a path, and the
/// directory a caller hashed is the one whose manifest declared this id.
pub fn pinned_connector_dir_sha256(connector_id: &str) -> Option<&'static str> {
    CONNECTOR_DIR_SHA256S
        .iter()
        .find(|(id, _)| *id == connector_id)
        .map(|(_, sha)| *sha)
}

/// Tests only: the connector ids this checkout ships, sorted, found by reading the folders
/// under the repository's `connectors/`. A test that runs a rule over "every shipped
/// connector" loops over this, so harness #4 is covered the day its folder lands and no test
/// names a connector by hand.
///
/// It panics unless the folders are exactly the ids [`CONNECTOR_DIR_SHA256S`] pins: a folder
/// that `scripts/pin-manifests.sh` did not pin, or a pin whose folder is gone, is the mistake
/// to fix first, before any per-connector rule runs. Only real directories count, so a stray
/// file such as `.DS_Store` is not a connector; `tests/it/harness_conformance.rs` refuses a
/// symlinked folder and a folder without a `harness.toml`.
///
/// Here, behind `test-hooks`, because both the protocol's and the CLI's tests use it and no
/// crate is shared by the two test trees except this one. The path is this crate's own
/// source location, which is why a shipped build never compiles it.
#[cfg(any(test, feature = "test-hooks"))]
pub fn shipped_connector_ids() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../connectors");
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut folders: Vec<String> = entries
        .map(|e| e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    let pinned: Vec<&str> = CONNECTOR_DIR_SHA256S.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        folders,
        pinned,
        "the folders under {} are not the ids pins/connector-dirs.txt pins: run \
         `sh scripts/pin-manifests.sh` and rebuild",
        dir.display()
    );
    folders
}

/// One `(connector_id, sha256)` pin table: the shape of [`CONNECTOR_DIR_SHA256S`] and of the
/// verifier's V1 manifest table.
pub type PinTable = &'static [(&'static str, &'static str)];

/// Whether `sha256` is `table`'s pin for `connector_id`: the one comparison of a pin in the tree
/// (the daemon's preflight, V13's finding, the CLI's `pin_state` and
/// [`connector_dir_is_pinned`] all call it).
///
/// By the DECLARED id, first row wins, and hex case is not meaningful, so a digest pasted in
/// uppercase names the same bytes. `false` when the table has no row for the id: an unpinned
/// connector is not a pinned one.
pub fn is_pinned(table: PinTable, connector_id: &str, sha256: &str) -> bool {
    table
        .iter()
        .find(|(id, _)| *id == connector_id)
        .is_some_and(|(_, want)| want.eq_ignore_ascii_case(sha256))
}

/// Whether `sha256` is this build's pinned digest for `connector_id`'s directory:
/// [`is_pinned`] over [`CONNECTOR_DIR_SHA256S`].
pub fn connector_dir_is_pinned(connector_id: &str, sha256: &str) -> bool {
    is_pinned(CONNECTOR_DIR_SHA256S, connector_id, sha256)
}

/// Whether a discovered connector is the one build-pinned directory for its id, or an
/// unpinned development connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectorFolderAdmission {
    Pinned,
    Unpinned,
}

/// Admit one discovered connector folder from the identity and digest captured by its one load.
///
/// A shipped connector has one identity in all three places: pin-table id, immediate-child folder
/// name, and declared manifest id; its digest must be that row's digest. If any one of folder, id
/// or digest names a shipped connector, all four facts must agree. This makes a byte-identical
/// copy under an alias a refusal rather than a second name for trusted bytes. A connector for
/// which none of the three facts names a pin is development/unpinned.
pub(crate) fn connector_folder_admission(
    folder_name: &OsStr,
    declared_id: &str,
    connector_dir_sha256: &str,
) -> Result<ConnectorFolderAdmission, ProtocolError> {
    let folder = folder_name.to_str().ok_or_else(|| ProtocolError::Field {
        field: "connector_folder".into(),
        reason: "a connector folder name is not UTF-8, so it cannot equal a pinned id".into(),
    })?;
    let folder_pin = CONNECTOR_DIR_SHA256S
        .iter()
        .find_map(|(id, _)| (*id == folder).then_some(*id));
    let declared_pin = CONNECTOR_DIR_SHA256S
        .iter()
        .find_map(|(id, _)| (*id == declared_id).then_some(*id));
    let digest_pin = CONNECTOR_DIR_SHA256S.iter().find_map(|(id, digest)| {
        digest
            .eq_ignore_ascii_case(connector_dir_sha256)
            .then_some(*id)
    });

    match (folder_pin, declared_pin, digest_pin) {
        (None, None, None) => Ok(ConnectorFolderAdmission::Unpinned),
        (Some(folder_id), Some(declared_id), Some(digest_id))
            if folder_id == declared_id && declared_id == digest_id =>
        {
            Ok(ConnectorFolderAdmission::Pinned)
        }
        (folder_pin, declared_pin, digest_pin) => {
            let pinned_id = declared_pin
                .or(folder_pin)
                .or(digest_pin)
                .unwrap_or("unknown");
            Err(ProtocolError::Field {
                field: "connector_folder".into(),
                reason: format!(
                    "build-pinned connector `{pinned_id}` must use folder `{pinned_id}`, declare \
                     that id, and contain its exact pinned bytes; found folder `{folder}`, \
                     declared id `{declared_id}`, digest `{connector_dir_sha256}`"
                ),
            })
        }
    }
}

/// The run-time twin of the build-time parser: the same scanner, with the line number of the
/// first refusal. Returns `(id, sha256)` pairs in file order.
///
/// `scripts/pin-manifests.sh` writes the file; this is what the tests (and anything that
/// wants to read a candidate file) use to learn why a file would not build.
pub fn parse_pins_text(text: &str) -> Result<Vec<(String, String)>, PinsError> {
    let bytes = text.as_bytes();
    // At most one pin per line; one extra row so an empty file allocates something sane.
    let lines = bytes.iter().filter(|b| **b == b'\n').count() + 1;
    let mut rows = vec![ZERO_ROW; lines];
    let n = scan(bytes, &mut rows, true)?;
    Ok(rows[..n]
        .iter()
        .map(|r| {
            (
                text[r.id_start..r.id_start + r.id_len].to_string(),
                text[r.sha_start..r.sha_start + SHA_LEN].to_string(),
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn problem(text: &str) -> (usize, PinProblem) {
        let e = parse_pins_text(text).expect_err("the file must be refused");
        (e.line, e.problem)
    }

    #[test]
    fn a_well_formed_file_parses_with_leading_comments() {
        let text = format!("# one\n# two\nalpha-cli {A}\nbeta-cli {B}\n");
        let pins = parse_pins_text(&text).expect("well formed");
        assert_eq!(
            pins,
            vec![
                ("alpha-cli".to_string(), A.to_string()),
                ("beta-cli".to_string(), B.to_string())
            ]
        );
    }

    #[test]
    fn the_committed_file_is_what_the_build_embedded() {
        let pins = parse_pins_text(DATA).expect("the committed file is well formed");
        assert_eq!(pins.len(), CONNECTOR_DIR_SHA256S.len());
        for ((id, sha), (cid, csha)) in pins.iter().zip(CONNECTOR_DIR_SHA256S) {
            assert_eq!((id.as_str(), sha.as_str()), (*cid, *csha));
        }
    }

    #[test]
    fn lookups_read_the_table_by_declared_id() {
        let (id, sha) = CONNECTOR_DIR_SHA256S[0];
        assert_eq!(pinned_connector_dir_sha256(id), Some(sha));
        assert!(connector_dir_is_pinned(id, &sha.to_ascii_uppercase()));
        assert!(!connector_dir_is_pinned(id, &"0".repeat(64)));
        assert_eq!(pinned_connector_dir_sha256("no-such-cli"), None);
        assert!(!connector_dir_is_pinned("no-such-cli", sha));
    }

    #[test]
    fn folder_admission_binds_a_shipped_digest_to_its_pinned_id_and_folder() {
        use std::ffi::OsStr;

        let (id, digest) = CONNECTOR_DIR_SHA256S[0];
        assert_eq!(
            connector_folder_admission(OsStr::new(id), id, digest).unwrap(),
            ConnectorFolderAdmission::Pinned
        );

        let alias = connector_folder_admission(OsStr::new("alias"), id, digest)
            .expect_err("byte-identical shipped bytes under an alias are not a second connector");
        assert!(alias.to_string().contains(id), "{alias}");
        assert!(alias.to_string().contains("folder"), "{alias}");

        let renamed = connector_folder_admission(OsStr::new(id), "unshipped-dev", &"ab".repeat(32))
            .expect_err("a pinned folder cannot declare a development identity");
        assert!(renamed.to_string().contains(id), "{renamed}");

        assert_eq!(
            connector_folder_admission(
                OsStr::new("unshipped-dev"),
                "unshipped-dev",
                &"ab".repeat(32),
            )
            .unwrap(),
            ConnectorFolderAdmission::Unpinned
        );
    }

    #[test]
    fn an_empty_or_comment_only_file_pins_nothing() {
        assert_eq!(problem(""), (0, PinProblem::NoPins));
        assert_eq!(problem("# only a comment\n"), (1, PinProblem::NoPins));
    }

    #[test]
    fn a_missing_trailing_newline_is_refused() {
        assert_eq!(
            problem(&format!("alpha-cli {A}")),
            (1, PinProblem::NoTrailingNewline)
        );
    }

    #[test]
    fn a_blank_line_is_refused() {
        assert_eq!(
            problem(&format!("alpha-cli {A}\n\nbeta-cli {B}\n")),
            (2, PinProblem::BlankLine)
        );
    }

    #[test]
    fn crlf_is_refused() {
        assert_eq!(
            problem(&format!("alpha-cli {A}\r\n")),
            (1, PinProblem::CarriageReturn)
        );
    }

    #[test]
    fn a_comment_after_a_pin_is_refused() {
        assert_eq!(
            problem(&format!("alpha-cli {A}\n# late\n")),
            (2, PinProblem::CommentAfterPins)
        );
    }

    #[test]
    fn a_malformed_id_is_refused() {
        for bad in [
            "Alpha",
            "a",
            "1alpha",
            "alpha_cli",
            "alpha.cli",
            &"a".repeat(49),
        ] {
            assert_eq!(
                problem(&format!("{bad} {A}\n")),
                (1, PinProblem::BadId),
                "{bad}"
            );
        }
        // The two bounds themselves are fine.
        assert!(parse_pins_text(&format!("ab {A}\n")).is_ok());
        assert!(parse_pins_text(&format!("{} {A}\n", "a".repeat(48))).is_ok());
    }

    #[test]
    fn the_separator_is_exactly_one_space() {
        assert_eq!(
            problem(&format!("alpha-cli  {A}\n")),
            (1, PinProblem::BadSeparator)
        );
        assert_eq!(problem("alpha-cli \n"), (1, PinProblem::BadSeparator));
        assert_eq!(problem("alpha-cli\n"), (1, PinProblem::BadSeparator));
        assert_eq!(
            problem(&format!("alpha-cli\t{A}\n")),
            (1, PinProblem::BadId)
        );
    }

    #[test]
    fn a_malformed_digest_is_refused() {
        let upper = A.replace('1', "A");
        for bad in [
            upper.as_str(),
            &A[..63],
            &format!("{A}0"),
            &A.replace('1', "g"),
            &format!("{A} "),
        ] {
            assert_eq!(
                problem(&format!("alpha-cli {bad}\n")),
                (1, PinProblem::BadDigest),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_unsorted_file_is_refused() {
        assert_eq!(
            problem(&format!("beta-cli {B}\nalpha-cli {A}\n")),
            (2, PinProblem::NotSorted)
        );
        // Byte order: `-` sorts before a digit and a letter, a prefix before its extension.
        assert!(parse_pins_text(&format!("ab {A}\nab-c {A}\nab0 {A}\nabc {A}\n")).is_ok());
        assert_eq!(
            problem(&format!("abc {A}\nab-c {A}\n")),
            (2, PinProblem::NotSorted)
        );
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        assert_eq!(
            problem(&format!("alpha-cli {A}\nalpha-cli {B}\n")),
            (2, PinProblem::Duplicate)
        );
    }

    #[test]
    fn every_problem_has_a_distinct_description() {
        let all = [
            PinProblem::NoPins,
            PinProblem::NoTrailingNewline,
            PinProblem::BlankLine,
            PinProblem::CarriageReturn,
            PinProblem::CommentAfterPins,
            PinProblem::BadId,
            PinProblem::BadSeparator,
            PinProblem::BadDigest,
            PinProblem::NotSorted,
            PinProblem::Duplicate,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for p in all {
            assert!(seen.insert(p.describe()), "{p:?} repeats a description");
        }
        let shown = PinsError {
            line: 3,
            problem: PinProblem::BadId,
        }
        .to_string();
        assert!(
            shown.contains("line 3") && shown.contains("^[a-z]"),
            "{shown}"
        );
    }

    /// The const parser and the run-time twin are one scanner: parse a literal in a const
    /// context and compare with the twin's answer over the same text.
    #[test]
    fn the_const_parser_agrees_with_its_twin() {
        const TEXT: &str = "# c\nalpha-cli 1111111111111111111111111111111111111111111111111111111111111111\nzeta-cli 2222222222222222222222222222222222222222222222222222222222222222\n";
        const COUNT: usize = count_pins(TEXT);
        const PARSED: [(&str, &str); COUNT] = parse_pins::<COUNT>(TEXT);
        let twin = parse_pins_text(TEXT).unwrap();
        assert_eq!(COUNT, 2);
        for ((id, sha), (tid, tsha)) in PARSED.iter().zip(&twin) {
            assert_eq!((*id, *sha), (tid.as_str(), tsha.as_str()));
        }
    }
}
