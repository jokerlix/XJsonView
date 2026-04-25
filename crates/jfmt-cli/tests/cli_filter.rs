use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn jfmt() -> Command {
    Command::cargo_bin("jfmt").unwrap()
}

#[test]
fn streaming_array_select() {
    jfmt()
        .args(["filter", "select(.x > 1)", fixture("filter_array.json").to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""x":2"#))
        .stdout(predicate::str::contains(r#""name":"b""#))
        .stdout(predicate::str::contains(r#""x":3"#))
        .stdout(predicate::str::contains(r#""name":"c""#))
        .stdout(predicate::str::contains(r#""x":1"#).not())
        .stderr(predicate::str::contains("streaming mode"));
}

#[test]
fn ndjson_select_skips_lines() {
    jfmt()
        .args([
            "filter",
            "--ndjson",
            r#"select(.level == "error")"#,
            fixture("filter_lines.ndjson").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(r#"{"i":1,"level":"error"}
{"i":3,"level":"error"}
"#);
}

#[test]
fn ndjson_multi_output_expands() {
    jfmt()
        .args([
            "filter",
            "--ndjson",
            ".i, .level",
            fixture("filter_lines.ndjson").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("0\n\"info\""))
        .stdout(predicate::str::contains("1\n\"error\""));
}

#[test]
fn aggregate_fails_with_exit_2() {
    jfmt()
        .args(["filter", "length"])
        .write_stdin("[1,2,3]")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("length"))
        .stderr(
            predicate::str::contains("--ndjson")
                .or(predicate::str::contains("--materialize")),
        );
}

#[test]
fn parse_error_fails_with_exit_2() {
    jfmt()
        .args(["filter", "not a valid )("])
        .write_stdin("[]")
        .assert()
        .code(2);
}

#[test]
fn runtime_error_default_exit_0() {
    jfmt()
        .args(["filter", "--ndjson", ".x + 1"])
        .write_stdin("{\"x\":\"a\"}\n{\"x\":2}\n")
        .assert()
        .success()
        .stdout("3\n")
        .stderr(predicate::str::contains("error"));
}

#[test]
fn runtime_error_strict_exit_1() {
    jfmt()
        .args(["filter", "--ndjson", "--strict", ".x + 1"])
        .write_stdin("{\"x\":\"a\"}\n{\"x\":2}\n")
        .assert()
        .code(1);
}

#[test]
fn threads_parity_serial_vs_parallel() {
    let mut input = String::new();
    for i in 0..500 {
        input.push_str(&format!("{{\"i\":{i}}}\n"));
    }

    let s1 = jfmt()
        .args([
            "--threads",
            "1",
            "filter",
            "--ndjson",
            "select(.i % 7 == 0)",
        ])
        .write_stdin(input.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s4 = jfmt()
        .args([
            "--threads",
            "4",
            "filter",
            "--ndjson",
            "select(.i % 7 == 0)",
        ])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(s1, s4);
}

#[test]
fn pretty_with_ndjson_is_rejected() {
    jfmt()
        .args(["filter", "--ndjson", "--pretty", "."])
        .write_stdin("{}")
        .assert()
        .failure();
}
