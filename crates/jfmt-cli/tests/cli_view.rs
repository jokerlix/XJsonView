use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn view_help_lists_subcommand() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("view"));
}

#[test]
fn view_with_missing_file_errors_clearly() {
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.args(["view", "definitely-does-not-exist.json"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("definitely-does-not-exist.json"));
}

#[test]
fn view_with_existing_file_attempts_to_spawn() {
    // We can't actually verify the GUI launches in a unit test, but we can
    // verify the command resolves the binary and produces a "not found"
    // diagnostic mentioning the search paths it checked.
    let mut cmd = Command::cargo_bin("jfmt").unwrap();
    cmd.env_remove("PATH"); // force the "binary not found" path
    cmd.args(["view", "Cargo.toml"]);
    cmd.assert()
        .failure()
        .stderr(
            predicate::str::contains("jfmt-viewer")
                .and(predicate::str::contains("could not find")),
        );
}
