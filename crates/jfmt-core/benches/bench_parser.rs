//! Criterion benchmark: EventReader throughput on a 1 MiB JSON fixture.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
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

fn parse_1mb(c: &mut Criterion) {
    let bytes = fixture_1mb();
    let mut group = c.benchmark_group("parser/event_reader");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("1MB", |b| {
        b.iter(|| {
            let mut r = EventReader::new(black_box(&bytes[..]));
            while let Some(ev) = r.next_event().unwrap() {
                black_box(ev);
            }
        });
    });
    group.finish();
}

criterion_group!(benches, parse_1mb);
criterion_main!(benches);
