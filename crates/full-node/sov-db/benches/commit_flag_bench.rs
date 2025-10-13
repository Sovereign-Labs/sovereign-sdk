extern crate criterion;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sov_db::commit_flag::{CommitFlag, CommitStatus};
use std::os::macos::raw::stat;

/// Benchmark write_status() with Completed status
fn bench_write_completed(c: &mut Criterion) {
    let tempdir = tempfile::tempdir().unwrap();
    let flag = CommitFlag::new(tempdir.path());

    c.bench_function("commit_flag_write_completed", |b| {
        b.iter(|| {
            let result = black_box(flag.write_status(CommitStatus::Completed));
            result.unwrap();
        });
    });
}

/// Benchmark write_status() with InProgress status
fn bench_write_in_progress(c: &mut Criterion) {
    let tempdir = tempfile::tempdir().unwrap();
    let flag = CommitFlag::new(tempdir.path());
    let root_hash = [0xAB; 32];

    c.bench_function("commit_flag_write_in_progress", |b| {
        b.iter(|| {
            let result = black_box(flag.write_status(CommitStatus::InProgress(root_hash)));
            result.unwrap();
        });
    });
}

/// Benchmark read_status()
fn bench_read_status(c: &mut Criterion) {
    let tempdir = tempfile::tempdir().unwrap();
    let flag = CommitFlag::new(tempdir.path());

    // Write initial status
    flag.write_status(CommitStatus::Completed).unwrap();

    c.bench_function("commit_flag_read_status", |b| {
        b.iter(|| {
            let status = black_box(flag.read_status());
            status.unwrap();
        });
    });
}

/// Benchmark full write-then-read cycle (simulates hot path)
fn bench_write_read_cycle(c: &mut Criterion) {
    let tempdir = tempfile::tempdir().unwrap();
    let flag = CommitFlag::new(tempdir.path());
    let root_hash = [0xCD; 32];

    c.bench_function("commit_flag_write_read_cycle", |b| {
        b.iter(|| {
            // Simulate commit flow: write InProgress, then Completed
            let result = flag.write_status(CommitStatus::InProgress(root_hash));
            black_box(result).unwrap();
            black_box(flag.read_status().unwrap());
            let result = flag.write_status(CommitStatus::Completed);
            black_box(result).unwrap();
            black_box(flag.read_status().unwrap());
        });
    });
}

/// Benchmark alternating writes (simulates realistic usage pattern)
fn bench_alternating_writes(c: &mut Criterion) {
    let tempdir = tempfile::tempdir().unwrap();
    let flag = CommitFlag::new(tempdir.path());
    let root_hash = [0xEF; 32];

    c.bench_function("commit_flag_alternating_writes", |b| {
        b.iter(|| {
            let result = flag.write_status(CommitStatus::InProgress(root_hash));
            black_box(result).unwrap();
            let result = flag.write_status(CommitStatus::Completed);
            black_box(result).unwrap();
        });
    });
}

criterion_group!(
    benches,
    bench_write_completed,
    bench_write_in_progress,
    bench_read_status,
    bench_write_read_cycle,
    bench_alternating_writes
);
criterion_main!(benches);
