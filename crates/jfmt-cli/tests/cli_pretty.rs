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
fn pretty_indent_2_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn pretty_indent_4_matches_golden() {
    let expected = fs::read_to_string(fixture("simple.pretty4.json")).unwrap();
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg("--indent")
        .arg("4")
        .arg(fixture("simple.json"))
        .assert()
        .success()
        .stdout(predicate::eq(expected));
}

#[test]
fn pretty_from_stdin() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .write_stdin("[1,2]")
        .assert()
        .success()
        .stdout("[\n  1,\n  2\n]\n");
}

#[test]
fn pretty_writes_to_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.json");
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg(fixture("simple.json"))
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    assert_eq!(fs::read_to_string(out).unwrap(), expected);
}

#[test]
fn pretty_roundtrips_gzip() {
    let dir = tempfile::tempdir().unwrap();
    let gz_in = dir.path().join("simple.json.gz");
    let raw = fs::read(fixture("simple.json")).unwrap();
    let mut enc = flate2::write::GzEncoder::new(
        fs::File::create(&gz_in).unwrap(),
        flate2::Compression::default(),
    );
    use std::io::Write;
    enc.write_all(&raw).unwrap();
    enc.finish().unwrap();

    let gz_out = dir.path().join("out.json.gz");
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg(&gz_in)
        .arg("-o")
        .arg(&gz_out)
        .assert()
        .success();

    let decoded = {
        let mut d = flate2::read::MultiGzDecoder::new(fs::File::open(&gz_out).unwrap());
        let mut s = String::new();
        use std::io::Read;
        d.read_to_string(&mut s).unwrap();
        s
    };
    let expected = fs::read_to_string(fixture("simple.pretty2.json")).unwrap();
    assert_eq!(decoded, expected);
}

#[test]
fn pretty_syntax_error_exits_2() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .write_stdin("{not json}")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("syntax error"));
}

#[test]
fn pretty_missing_file_exits_1() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .arg("pretty")
        .arg("no-such-file.json")
        .assert()
        .code(1);
}
