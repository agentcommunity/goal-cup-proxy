//! Hermes lane B fix round 1, secret review M-2: the `ps` children of the process checks run with
//! an empty environment. `goalcup` itself holds the provider key in its environment for a Hermes
//! run, and a child that inherits it is a copy of it the process-table helper has no use for.

#[test]
fn every_ps_child_starts_with_an_empty_environment() {
    let source = include_str!("../../src/process.rs");
    let mut sites = 0;
    for (at, _) in source.match_indices("Command::new(\"/bin/ps\")") {
        sites += 1;
        let next: String = source[at..]
            .lines()
            .skip(1)
            .take(2)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            next.contains(".env_clear()"),
            "a `/bin/ps` child inherits the environment:\n{}",
            &source[at..at + 120]
        );
    }
    assert!(sites >= 3, "{sites} sites found");
}
