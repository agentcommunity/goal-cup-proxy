//! PRD-04 sections 2 and 7: row patterns with `**`, the first-match rule and the path ID.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use goalcup::native::pattern::{Pattern, Row, first_match, parse, path_id};

fn p(s: &str) -> Pattern {
    parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
}

fn row(root: &Path, pattern: &Pattern) -> Row {
    let (name, root, pattern) = ("r".into(), root.into(), pattern.clone());
    Row {
        name,
        root,
        pattern,
    }
}

#[test]
fn any_depth_matches_zero_and_many_folders() {
    let pat = p("a/**/b.jsonl");
    for rel in ["a/b.jsonl", "a/x/b.jsonl", "a/x/y/z/b.jsonl"] {
        assert!(pat.matches(Path::new(rel)), "{rel}");
    }
    for rel in ["b.jsonl", "x/a/b.jsonl", "a/x/c.jsonl", "a/x"] {
        assert!(!pat.matches(Path::new(rel)), "{rel}");
    }
    // `**` never names a file by itself: `a/**/*` does not match a file called `a`.
    assert!(!p("a/**/*").matches(Path::new("a")));
}

#[test]
fn a_glob_never_matches_across_a_slash() {
    assert!(!p("*.jsonl").matches(Path::new("a/b.jsonl")));
    assert!(!p("a*").matches(Path::new("ab/c")));
    assert!(!p("a?c").matches(Path::new("a/c")));
    assert!(p("a?c/*").matches(Path::new("abc/d")));
}

#[test]
fn a_star_matches_a_name_that_is_not_utf8() {
    let name = OsStr::from_bytes(b"x\xff.jsonl");
    assert!(p("*.jsonl").matches(Path::new(name)));
    assert!(p("**/*").matches(&Path::new("d").join(name)));
}

#[test]
fn bad_patterns_are_refused() {
    for bad in [
        "", "/a", "/", "a//b", "a/", ".", "a/./b", "..", "a/../b", "{a,b}", "a/{b", "a}", "a\0b",
        "a**", "**b", "x**y", "a/***", "**", "a/**",
    ] {
        assert!(parse(bad).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn deep_patterns_have_no_cap() {
    let deep: Vec<String> = (0..40).map(|i| format!("d{i}")).collect();
    let pat = p(&deep.join("/"));
    assert!(pat.matches(Path::new(&deep.join("/"))));
}

#[test]
fn may_enter_follows_the_pattern() {
    let pat = p("sessions/**/*.jsonl");
    assert!(pat.may_enter(Path::new("")));
    assert!(pat.may_enter(Path::new("sessions")));
    assert!(pat.may_enter(Path::new("sessions/a/b/c/d/e")));
    assert!(!pat.may_enter(Path::new("other")));
    assert!(!pat.may_enter(Path::new("other/sessions")));

    let flat = p("a/b.jsonl");
    assert!(flat.may_enter(Path::new("a")));
    assert!(!flat.may_enter(Path::new("a/b.jsonl")));
    assert!(!flat.may_enter(Path::new("b")));
}

#[test]
fn the_first_matching_row_wins() {
    let root = Path::new("/home/h/projects");
    let all = p("**/*.jsonl");
    let sub = p("sub/*.jsonl");
    let file = Path::new("/home/h/projects/sub/x.jsonl");

    let rows = [row(root, &all), row(root, &sub)];
    assert_eq!(
        first_match(&rows, file),
        Some((0, Path::new("sub/x.jsonl")))
    );
    let rows = [row(root, &sub), row(root, &all)];
    assert_eq!(
        first_match(&rows, file),
        Some((0, Path::new("sub/x.jsonl")))
    );

    // One root inside the other: the first row still wins, and the ID is relative to its root.
    let outer = Path::new("/home/h");
    let one = p("*/*.jsonl");
    let rows = [row(root, &one), row(outer, &all)];
    assert_eq!(
        first_match(&rows, file),
        Some((0, Path::new("sub/x.jsonl")))
    );
    let rows = [row(outer, &all), row(root, &one)];
    assert_eq!(
        first_match(&rows, file),
        Some((0, Path::new("projects/sub/x.jsonl")))
    );

    // A root is a whole-component prefix, never a string prefix.
    let rows = [row(Path::new("/home/h/proj"), &all)];
    assert_eq!(first_match(&rows, file), None);
}

#[test]
fn path_id_escapes_and_never_holds_the_root() {
    let rel = Path::new(OsStr::from_bytes(b"a\\b/c\x01d/e\xffz.jsonl"));
    assert_eq!(path_id("row", rel), "row:a\\\\b/c\\u{1}d/e\\xffz.jsonl");

    let all = p("**/*.jsonl");
    let root = Path::new("/home/secret-root");
    let file = Path::new("/home/secret-root/s/t.jsonl");
    let (row, rel) = first_match(&[row(root, &all)], file).expect("match");
    assert_eq!(row, 0);
    let id = path_id("transcripts", rel);
    assert_eq!(id, "transcripts:s/t.jsonl");
    assert!(!id.contains("secret-root"));
}
