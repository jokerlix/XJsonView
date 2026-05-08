use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn view_subcommand_exists_and_prints_placeholder() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["view", "some.json"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("GUI viewer not yet bundled"));
}

#[test]
fn view_help_lists_subcommand() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("view"));
}
