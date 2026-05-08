//! End-to-end tests for `jfmt convert`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/convert")
        .join(name)
}

fn read_fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap_or_else(|e| panic!("read {name}: {e}"))
}

#[test]
fn xml_to_json_atom_feed_matches_golden() {
    let xml = read_fixture("atom_feed.xml");
    let golden = read_fixture("atom_feed.json");
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "xml", "--to", "json"])
        .write_stdin(xml)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out_v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let golden_v: serde_json::Value = serde_json::from_slice(&golden).unwrap();
    assert_eq!(out_v, golden_v);
}

#[test]
fn xml_to_json_via_file_extension() {
    let path = fixture("atom_feed.xml");
    Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""feed":"#));
}

#[test]
fn json_to_xml_round_trip_data_records() {
    let json = read_fixture("data_records.json");
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "json", "--to", "xml"])
        .write_stdin(json)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out_str = String::from_utf8(out).unwrap();
    assert!(out_str.contains("<record"));
    assert!(out_str.contains("alice"));
}

#[test]
fn mixed_content_concatenates_text() {
    let xml = read_fixture("mixed_content.xml");
    let golden = read_fixture("mixed_content.json");
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "xml", "--to", "json"])
        .write_stdin(xml)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out_v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let golden_v: serde_json::Value = serde_json::from_slice(&golden).unwrap();
    assert_eq!(out_v, golden_v);
}

#[test]
fn noncontiguous_siblings_warns_default() {
    let xml = read_fixture("noncontiguous_siblings.xml");
    Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "xml", "--to", "json"])
        .write_stdin(xml)
        .assert()
        .success()
        .stderr(predicate::str::contains("non-contiguous"));
}

#[test]
fn noncontiguous_siblings_strict_exits_34() {
    let xml = read_fixture("noncontiguous_siblings.xml");
    Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "xml", "--to", "json", "--strict"])
        .write_stdin(xml)
        .assert()
        .code(34);
}

#[test]
fn array_rule_collapses_record() {
    let xml = read_fixture("data_records.xml");
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args([
            "convert",
            "--from",
            "xml",
            "--to",
            "json",
            "--array-rule",
            "root.record.name",
        ])
        .write_stdin(xml)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // root.record.name should now be a bare object, not an array.
    let name = &v["root"][0]["record"][0]["name"];
    assert!(name.is_object(), "expected object, got: {name}");
}

#[test]
fn json_to_xml_root_wraps_multi_key() {
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "json", "--to", "xml", "--root", "doc"])
        .write_stdin(r#"{"a":1,"b":2}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "<doc><a>1</a><b>2</b></doc>"
    );
}

#[test]
fn json_to_xml_xml_decl() {
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "json", "--to", "xml", "--xml-decl"])
        .write_stdin(r#"{"a":"v"}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(
        s.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#),
        "got: {s}"
    );
}

#[test]
fn json_to_xml_pretty() {
    let out = Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "json", "--to", "xml", "--pretty"])
        .write_stdin(r#"{"a":{"b":"v"}}"#)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains('\n'), "expected newlines, got: {s}");
}

#[test]
fn unknown_extension_errors() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "/tmp/nonexistent.txt"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("infer"));
}

#[test]
fn invalid_xml_input_exits_21() {
    Command::cargo_bin("jfmt")
        .unwrap()
        .args(["convert", "--from", "xml", "--to", "json"])
        .write_stdin("<a><b></c>")
        .assert()
        .code(21);
}
