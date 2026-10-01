use std::hint::black_box;
use std::time::{Duration, Instant};

use banshee::bench::{self, LONG, MEDIUM, SAMPLE_RATE, SHORT};
use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};

fn timed<T>(iters: u64, mut body: impl FnMut() -> T) -> Duration {
    let mut total = Duration::ZERO;
    for _ in 0..iters {
        let start = Instant::now();
        let output = body();
        total += start.elapsed();
        drop(black_box(output));
    }
    total
}

const TEXTS: [(&str, &str); 3] = [("short", SHORT), ("medium", MEDIUM), ("long", LONG)];

fn text_path(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_path");
    for (name, text) in TEXTS {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_function(name, |b| b.iter(|| bench::text_path(black_box(text))));
    }
    group.finish();
}

fn g2p(c: &mut Criterion) {
    let g2p = bench::english_g2p();
    let mut group = c.benchmark_group("g2p");
    for (name, text) in TEXTS {
        let sentences = bench::text_path(text);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                for sentence in &sentences {
                    black_box(g2p.g2p(black_box(sentence)).unwrap());
                }
            })
        });
    }
    group.finish();
}

fn kokoro(c: &mut Criterion) {
    if let Some(missing) = bench::missing_model() {
        eprintln!("the kokoro group is skipped: {missing}");
        return;
    }
    eprintln!("kokoro: {}", bench::run_facts());

    let mut group = c.benchmark_group("kokoro");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);

    // About ten times each bench's measured iteration, so Criterion runs one iteration per sample.
    group.measurement_time(Duration::from_secs(2));
    group.bench_function("warm_cache_load", |b| {
        b.iter_custom(|iters| timed(iters, || bench::engine().unwrap()))
    });

    group.measurement_time(Duration::from_secs(7));
    let short = bench::text_path(SHORT).remove(0);
    group.bench_function("first_speech", |b| {
        b.iter_custom(|iters| {
            timed(iters, || {
                let mut engine = bench::engine().unwrap();
                black_box(engine.synthesize(black_box(&short)).unwrap());
                engine
            })
        })
    });

    let mut engine = bench::engine().unwrap();
    for (name, text, seconds) in [
        ("synthesize_short", SHORT, 5),
        ("synthesize_long_windowed", LONG, 87),
    ] {
        let sentence = bench::text_path(text).remove(0);
        let samples = engine.synthesize(&sentence).unwrap().len();
        group.throughput(Throughput::Elements(samples as u64));
        group.measurement_time(Duration::from_secs(seconds));
        group.bench_function(name, |b| {
            b.iter(|| engine.synthesize(black_box(&sentence)).unwrap())
        });
    }
    group.finish();
    eprintln!("kokoro: realtime factor = elements per second / {SAMPLE_RATE}");
}

criterion_group!(benches, text_path, g2p, kokoro);
criterion_main!(benches);
