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
fn validate_ndjson_parallel_matches_serial_stats() {
    let dir = tempfile::tempdir().unwrap();
    let s1 = dir.path().join("s1.json");
    let s4 = dir.path().join("s4.json");

    for (path, threads) in [(&s1, "1"), (&s4, "4")] {
        Command::cargo_bin("jfmt")
            .unwrap()
            .arg("--threads")
            .arg(threads)
            .arg("validate")
            .arg("--ndjson")
            .arg("--stats-json")
            .arg(path)
            .arg(fixture("ndjson-many.ndjson"))
            .assert()
            .success();
    }

    let v1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&s1).unwrap()).unwrap();
    let v4: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&s4).unwrap()).unwrap();
    assert_eq!(v1, v4);
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

// ===== M5 — JSON Schema =====

fn jfmt() -> Command {
    Command::cargo_bin("jfmt").unwrap()
}

#[test]
fn schema_ndjson_default_continues_with_violations() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .success() // exit 0 by default
        .stderr(predicate::str::contains("schema:"));
}

#[test]
fn schema_ndjson_strict_exits_3() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .code(3);
}

#[test]
fn schema_streaming_array_validates_each_element() {
    jfmt()
        .args([
            "validate",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_array.json",
        ])
        .assert()
        .code(3) // 2 violations
        .stderr(predicate::str::contains("element"));
}

#[test]
fn schema_streaming_non_array_root_requires_materialize() {
    jfmt()
        .args([
            "validate",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a"}"#)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--materialize"));
}

#[test]
fn schema_materialize_whole_doc() {
    jfmt()
        .args([
            "validate",
            "-m",
            "--strict",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a"}"#) // missing age
        .assert()
        .code(3)
        .stderr(predicate::str::contains("required"));
}

#[test]
fn schema_materialize_passes_on_good_input() {
    jfmt()
        .args([
            "validate",
            "-m",
            "--schema",
            "tests/fixtures/schema_user.json",
        ])
        .write_stdin(r#"{"name":"a","age":1}"#)
        .assert()
        .success();
}

#[test]
fn schema_fail_fast_aborts_at_first() {
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--strict",
            "--fail-fast",
            "--schema",
            "tests/fixtures/schema_user.json",
            "tests/fixtures/schema_user_ndjson.ndjson",
        ])
        .assert()
        .code(3);
}

#[test]
fn schema_file_missing_exits_1() {
    jfmt()
        .args(["validate", "--schema", "tests/fixtures/nonexistent.json"])
        .write_stdin("[]")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("schema"));
}

#[test]
fn schema_file_invalid_json_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "not valid json").unwrap();
    jfmt()
        .args(["validate", "--schema"])
        .arg(&bad)
        .write_stdin("[]")
        .assert()
        .code(1);
}

#[test]
fn schema_file_invalid_schema_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad-schema.json");
    std::fs::write(&bad, r#"{"type":42}"#).unwrap();
    jfmt()
        .args(["validate", "--schema"])
        .arg(&bad)
        .write_stdin("[]")
        .assert()
        .code(1);
}

#[test]
fn schema_stats_json_includes_schema_fields() {
    let dir = tempfile::tempdir().unwrap();
    let stats_path = dir.path().join("stats.json");
    jfmt()
        .args([
            "validate",
            "--ndjson",
            "--schema",
            "tests/fixtures/schema_user.json",
            "--stats-json",
        ])
        .arg(&stats_path)
        .arg("tests/fixtures/schema_user_ndjson.ndjson")
        .assert()
        .success();
    let body = std::fs::read_to_string(&stats_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["schema_pass"].as_u64().unwrap() >= 1);
    assert!(v["schema_fail"].as_u64().unwrap() >= 1);
    assert!(v["top_violation_paths"].is_object());
}

#[test]
fn validate_materialize_conflicts_with_ndjson() {
    jfmt()
        .args(["validate", "-m", "--ndjson"])
        .write_stdin("[]")
        .assert()
        .code(2);
}

#[test]
fn validate_force_requires_materialize() {
    jfmt()
        .args(["validate", "--force"])
        .write_stdin("[]")
        .assert()
        .code(2);
}
