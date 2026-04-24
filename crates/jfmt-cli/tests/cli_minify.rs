use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn minify_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.min.json")).unwrap();
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("minify")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn minify_from_stdin_to_stdout() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("minify")
        .write_stdin("{ \"a\" :  [ 1 , 2 ] }")
        .assert()
        .success()
        .stdout("{\"a\":[1,2]}");
}

#[test]
fn minify_ndjson_parallel_matches_serial() {
    let serial = Command::cargo_bin("jfmt")
        .unwrap()
        .arg("--threads")
        .arg("1")
        .arg("minify")
        .arg("--ndjson")
        .arg(fixture("ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parallel = Command::cargo_bin("jfmt")
        .unwrap()
        .arg("--threads")
        .arg("4")
        .arg("minify")
        .arg("--ndjson")
        .arg(fixture("ndjson-many.ndjson"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(serial, parallel);
}

#[test]
fn minify_zstd_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let zst_out = dir.path().join("out.json.zst");
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("minify")
        .arg(fixture("simple.json"))
        .arg("-o")
        .arg(&zst_out)
        .assert()
        .success();
    let decoded = zstd::decode_all(fs::File::open(&zst_out).unwrap()).unwrap();
    let expected = fs::read_to_string(fixture("simple.min.json")).unwrap();
    assert_eq!(String::from_utf8(decoded).unwrap(), expected);
}
