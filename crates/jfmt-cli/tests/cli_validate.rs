use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn validate_good_exits_zero() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg(fixture("simple.json"))
        .assert()
        .success();
}

#[test]
fn validate_bad_exits_2_with_location() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg(fixture("bad.json"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("syntax error"))
        .stderr(predicate::str::contains("line"))
        .stderr(predicate::str::contains("column"));
}

#[test]
fn validate_stats_to_stderr() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--stats")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stderr(predicate::str::contains("records: 1 (1 valid, 0 invalid)"))
        .stderr(predicate::str::contains("top-level types:"))
        .stderr(predicate::str::contains("object: 1"));
}

#[test]
fn validate_stats_json_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("stats.json");
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--stats-json")
        .arg(&out)
        .arg(fixture("simple.json"))
        .assert()
        .success();

    let text = std::fs::read_to_string(&out).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["records"], 1);
    assert_eq!(v["valid"], 1);
    assert_eq!(v["max_depth"], 2);
    assert_eq!(v["top_level_types"]["object"], 1);
}

#[test]
fn validate_ndjson_good_exits_zero() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg(fixture("ndjson-good.ndjson"))
        .assert()
        .success();
}

#[test]
fn validate_ndjson_mixed_reports_line_2_and_exits_2() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg(fixture("ndjson-mixed.ndjson"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("line 2:"))
        .stderr(predicate::str::contains("line 1:").not())
        .stderr(predicate::str::contains("line 3:").not());
}

#[test]
fn validate_ndjson_fail_fast_stops_after_first_bad() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("two_bad.ndjson");
    let mut f = std::fs::File::create(&p).unwrap();
    writeln!(f, "{{bad1").unwrap();
    writeln!(f, "{{bad2").unwrap();
    writeln!(f, "1").unwrap();
    drop(f);

    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg("--fail-fast")
        .arg(&p)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("line 1:"))
        .stderr(predicate::str::contains("line 2:").not());
}

#[test]
fn validate_ndjson_stats_counts_valid_and_invalid() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("validate")
        .arg("--ndjson")
        .arg("--stats")
        .arg(fixture("ndjson-mixed.ndjson"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("records: 3 (2 valid, 1 invalid)"));
}
