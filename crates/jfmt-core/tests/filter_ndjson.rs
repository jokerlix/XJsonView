use jfmt_core::filter::{compile, run_ndjson, FilterOptions, Mode};
use std::io::Cursor;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);
impl SharedBuf {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}
impl std::io::Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run(expr: &str, input: &[u8], threads: usize, strict: bool) -> (String, usize) {
    let compiled = compile(expr, Mode::Streaming).unwrap();
    let buf = SharedBuf::new();
    let report = run_ndjson(
        Cursor::new(input.to_vec()),
        buf.clone(),
        compiled,
        threads,
        FilterOptions { strict },
    )
    .expect("run_ndjson");
    let s = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    (s, report.errors.len())
}

#[test]
fn select_skips_non_matching_lines() {
    let input = b"{\"x\":1}\n{\"x\":2}\n{\"x\":3}\n";
    let (out, errs) = run("select(.x > 1)", input, 1, false);
    assert_eq!(errs, 0);
    assert_eq!(out, "{\"x\":2}\n{\"x\":3}\n");
}

#[test]
fn comma_emits_two_lines_per_input() {
    let input = b"{\"a\":1,\"b\":2}\n";
    let (out, _) = run(".a, .b", input, 1, false);
    assert_eq!(out, "1\n2\n");
}

#[test]
fn empty_output_lines_are_omitted() {
    let input = b"{\"x\":1}\n{\"x\":-1}\n{\"x\":5}\n";
    let (out, errs) = run("select(.x > 0)", input, 1, false);
    assert_eq!(errs, 0);
    assert_eq!(out, "{\"x\":1}\n{\"x\":5}\n");
}

#[test]
fn type_error_default_continues() {
    let input = b"{\"x\":\"hi\"}\n{\"x\":2}\n";
    let (out, errs) = run(".x + 1", input, 1, false);
    assert_eq!(errs, 1);
    assert_eq!(out, "3\n");
}

#[test]
fn type_error_strict_aborts_first() {
    let input = b"{\"x\":\"hi\"}\n{\"x\":2}\n";
    // strict + fail_fast: at least one error reported; the second
    // line may or may not have been processed by the time cancel
    // propagates.
    let (_, errs) = run(".x + 1", input, 1, true);
    assert!(errs >= 1);
}

#[test]
fn parallel_matches_serial() {
    let mut input = Vec::new();
    for i in 0..200u32 {
        input.extend_from_slice(format!("{{\"i\":{i}}}\n").as_bytes());
    }
    let (s1, _) = run("select(.i % 3 == 0)", &input, 1, false);
    let (s4, _) = run("select(.i % 3 == 0)", &input, 4, false);
    assert_eq!(s1, s4);
}
