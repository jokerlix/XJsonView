//! Performance regression for the M10 viewer-core rewrite. Gated behind
//! `--features big-tests` so default `cargo test` stays fast.
//!
//! Run:
//!   cargo test -p jfmt-viewer-core --features big-tests \
//!       --test big_array --release

#![cfg(feature = "big-tests")]

use std::io::Write;
use std::time::Instant;

use jfmt_viewer_core::{NodeId, Session};

// ~25 MB of small object records. Big enough that the original O(N²)
// implementation would never finish; small enough that the current
// depth-tracking parser meets the < 500ms budget on commodity hardware.
// (A true subtree byte-skip is a follow-up; would unlock larger N.)
const N: usize = 100_000;

fn write_big_array() -> tempfile::NamedTempFile {
    let f = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .unwrap();
    let mut w = std::io::BufWriter::new(f.reopen().unwrap());
    write!(w, "[").unwrap();
    for i in 0..N {
        if i > 0 {
            write!(w, ",").unwrap();
        }
        write!(
            w,
            r#"{{"i":{i},"name":"row-{i}","tags":["a","b","c"],"nested":{{"x":{i},"y":{}}}}}"#,
            i * 2
        )
        .unwrap();
    }
    write!(w, "]").unwrap();
    w.flush().unwrap();
    drop(w);
    f
}

#[test]
fn open_and_paginate_under_500ms_per_call() {
    let f = write_big_array();
    let s = Session::open(f.path()).expect("open");

    let t0 = Instant::now();
    let head = s.get_children(NodeId::ROOT, 0, 200).expect("head page");
    let head_ms = t0.elapsed().as_millis();
    assert_eq!(head.total as usize, N);
    assert_eq!(head.items.len(), 200);
    assert!(
        head_ms < 500,
        "head pagination too slow: {head_ms} ms (target < 500)"
    );

    let t1 = Instant::now();
    let tail = s
        .get_children(NodeId::ROOT, (N as u32) - 100, 50)
        .expect("tail page");
    let tail_ms = t1.elapsed().as_millis();
    assert_eq!(tail.items.len(), 50);
    assert_eq!(tail.items[0].key, format!("{}", N - 100));
    assert_eq!(tail.items[49].key, format!("{}", N - 51));
    assert!(
        tail_ms < 500,
        "tail pagination too slow: {tail_ms} ms (target < 500)"
    );
}
