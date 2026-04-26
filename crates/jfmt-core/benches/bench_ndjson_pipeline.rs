//! Criterion benchmark: run_ndjson_pipeline at threads=1 vs threads=cores.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use jfmt_core::{run_ndjson_pipeline, LineError, NdjsonPipelineOptions, StatsCollector};
use std::io::Cursor;

fn fixture_ndjson(lines: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    for i in 0..lines {
        let line = format!(r#"{{"id":{i},"name":"record-{i}","tags":["x","y"],"active":true}}"#);
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
    }
    buf
}

fn pipeline_passthrough(c: &mut Criterion) {
    const LINES: usize = 100_000;
    let bytes = fixture_ndjson(LINES);
    let mut group = c.benchmark_group("ndjson_pipeline/passthrough");
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    for threads in [1usize, num_cpus::get_physical().max(1)] {
        group.bench_with_input(
            criterion::BenchmarkId::from_parameter(threads),
            &threads,
            |b, &threads| {
                b.iter(|| {
                    let opts = NdjsonPipelineOptions {
                        threads,
                        ..Default::default()
                    };
                    let report = run_ndjson_pipeline(
                        Cursor::new(bytes.clone()),
                        std::io::sink(),
                        |line: &[u8], c: &mut StatsCollector| -> Result<Vec<Vec<u8>>, LineError> {
                            c.begin_record();
                            c.end_record(true);
                            Ok(vec![line.to_vec()])
                        },
                        opts,
                    )
                    .unwrap();
                    black_box(report);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, pipeline_passthrough);
criterion_main!(benches);
