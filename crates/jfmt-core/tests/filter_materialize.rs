use jfmt_core::filter::static_check::Mode;
use jfmt_core::filter::{
    compile, run_materialize, FilterError, FilterOptions, FilterOutput,
};
use jfmt_core::PrettyConfig;

fn run(expr: &str, input: &str) -> String {
    let compiled = compile(expr, Mode::Materialize).expect("compile");
    let mut out = Vec::<u8>::new();
    run_materialize(
        input.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .expect("run_materialize");
    String::from_utf8(out).unwrap()
}

#[test]
fn group_by_returns_grouped_array() {
    let s = run(
        "group_by(.k)",
        r#"[{"k":"a","v":1},{"k":"b","v":2},{"k":"a","v":3}]"#,
    );
    // Two groups: [{k:a,v:1},{k:a,v:3}] and [{k:b,v:2}].
    assert!(s.starts_with('[') && s.ends_with(']'));
    assert!(s.contains(r#""k":"a","v":1"#) || s.contains(r#""v":1,"k":"a""#));
    assert!(s.contains(r#""k":"b","v":2"#) || s.contains(r#""v":2,"k":"b""#));
}

#[test]
fn add_sums_array() {
    let s = run("add", "[1,2,3,4]");
    assert_eq!(s, "10");
}

#[test]
fn unique_dedupes() {
    let s = run("unique", "[3,1,2,1,3]");
    assert_eq!(s, "[1,2,3]");
}

#[test]
fn pretty_uses_double_newline_separator() {
    let compiled = compile(".[]", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let cfg = PrettyConfig {
        indent: 2,
        ..PrettyConfig::default()
    };
    run_materialize(
        "[1,2,3]".as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Pretty(cfg),
        FilterOptions::default(),
    )
    .unwrap();
    let s = String::from_utf8(out).unwrap();
    // PrettyWriter formats scalars unchanged; values separated by \n\n.
    assert_eq!(s, "1\n\n2\n\n3");
}

#[test]
fn type_error_default_collected() {
    let compiled = compile(".x + 1", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_materialize(
        r#"{"x":"hi"}"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, FilterError::Runtime { .. }));
}

#[test]
fn type_error_strict_returns_err() {
    let compiled = compile(".x + 1", Mode::Materialize).unwrap();
    let mut out = Vec::<u8>::new();
    let err = run_materialize(
        r#"{"x":"hi"}"#.as_bytes(),
        &mut out,
        &compiled,
        FilterOutput::Compact,
        FilterOptions { strict: true },
    )
    .unwrap_err();
    assert!(matches!(err, FilterError::Runtime { .. }));
}
