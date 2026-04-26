//! Criterion benchmark: MinifyWriter and PrettyWriter throughput.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use jfmt_core::writer::{EventWriter, MinifyWriter, PrettyConfig, PrettyWriter};
use jfmt_core::EventReader;

fn fixture_1mb() -> Vec<u8> {
    let mut buf = b"[".to_vec();
    let record = br#"{"id":12345,"name":"alice","tags":["x","y"],"active":true}"#;
    while buf.len() + record.len() + 2 < 1024 * 1024 {
        if buf.len() > 1 {
            buf.push(b',');
        }
        buf.extend_from_slice(record);
    }
    buf.push(b']');
    buf
}

fn collect_events(input: &[u8]) -> Vec<jfmt_core::Event> {
    let mut r = EventReader::new(input);
    let mut events = Vec::new();
    while let Some(ev) = r.next_event().unwrap() {
        events.push(ev);
    }
    events
}

fn minify_1mb(c: &mut Criterion) {
    let bytes = fixture_1mb();
    let events = collect_events(&bytes);
    let mut group = c.benchmark_group("writer/minify");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("1MB", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(bytes.len());
            let mut w = MinifyWriter::new(&mut buf);
            for ev in &events {
                w.write_event(black_box(ev)).unwrap();
            }
            w.finish().unwrap();
            black_box(buf);
        });
    });
    group.finish();
}

fn pretty_1mb(c: &mut Criterion) {
    let bytes = fixture_1mb();
    let events = collect_events(&bytes);
    let mut group = c.benchmark_group("writer/pretty");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("1MB/indent2", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(bytes.len() * 2);
            let mut w = PrettyWriter::with_config(&mut buf, PrettyConfig::default());
            for ev in &events {
                w.write_event(black_box(ev)).unwrap();
            }
            w.finish().unwrap();
            black_box(buf);
        });
    });
    group.finish();
}

criterion_group!(benches, minify_1mb, pretty_1mb);
criterion_main!(benches);
