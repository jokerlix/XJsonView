use jfmt_core::filter::{compile, run_streaming, FilterOptions, FilterOutput};

fn run(expr: &str, input: &str) -> (String, jfmt_core::filter::StreamingReport) {
    let compiled = compile(expr).expect("compile");
    let mut out = Vec::<u8>::new();
    let report = run_streaming(
        input.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .expect("run_streaming");
    (String::from_utf8(out).unwrap(), report)
}

#[test]
fn array_select_filters_elements() {
    let (out, _) = run("select(.x > 1)", r#"[{"x":1},{"x":2},{"x":3}]"#);
    assert_eq!(out, r#"[{"x":2},{"x":3}]"#);
}

#[test]
fn array_identity_passes_through() {
    let (out, _) = run(".", r#"[1,2,3]"#);
    assert_eq!(out, "[1,2,3]");
}

#[test]
fn object_filter_drops_keys() {
    let (out, _) = run("select(. > 1)", r#"{"a":1,"b":2,"c":3}"#);
    assert!(out.contains(r#""b":2"#));
    assert!(out.contains(r#""c":3"#));
    assert!(!out.contains(r#""a":1"#));
    assert!(out.starts_with('{') && out.ends_with('}'));
}

#[test]
fn scalar_filter_passes_through() {
    let (out, _) = run("select(. > 0)", "5");
    assert_eq!(out, "5");
}

#[test]
fn scalar_filter_dropping() {
    let (out, _) = run("select(. > 0)", "-1");
    assert_eq!(out, "");
}

#[test]
fn array_multi_output_expands() {
    let (out, _) = run(".x, .y", r#"[{"x":1,"y":2}]"#);
    assert_eq!(out, "[1,2]");
}

#[test]
fn object_multi_output_records_runtime_error() {
    let (out, report) = run(".a, .b", r#"{"k":{"a":1,"b":2}}"#);
    assert_eq!(out, "{}");
    assert_eq!(report.runtime_errors.len(), 1);
}

#[test]
fn type_error_records_runtime_error_default() {
    let (out, report) = run(".x + 1", r#"[{"x":"hi"},{"x":2}]"#);
    assert_eq!(out, "[3]");
    assert_eq!(report.runtime_errors.len(), 1);
}

#[test]
fn type_error_strict_returns_err() {
    let compiled = compile(".x + 1").unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_streaming(
        r#"[{"x":"hi"}]"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions { strict: true },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        jfmt_core::filter::FilterError::Runtime { .. }
    ));
}
