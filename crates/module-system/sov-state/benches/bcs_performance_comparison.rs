use criterion::{black_box, criterion_group, criterion_main, Criterion};
use alloy_primitives::{B256, U256};
use revm::state::AccountInfo;
use serde::{Deserialize, Serialize};
use sov_state::codec::{BcsCodec, StateItemDecoder, StateItemEncoder};

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone, Default)]
pub struct DbAccount(pub AccountInfo);

fn bench_u64_decode(c: &mut Criterion) {
    let codec = BcsCodec;
    let value: u64 = 42;
    let encoded = codec.encode(&value);
    
    c.bench_function("u64_decode_hot", |b| {
        b.iter(|| {
            let decoded: u64 = codec.try_decode(black_box(&encoded)).unwrap();
            black_box(decoded);
        });
    });
}

fn bench_u256_decode(c: &mut Criterion) {
    let codec = BcsCodec;
    let value = U256::from(12345678901234567890u64);
    let encoded = codec.encode(&value);
    
    c.bench_function("u256_decode_hot", |b| {
        b.iter(|| {
            let decoded: U256 = codec.try_decode(black_box(&encoded)).unwrap();
            black_box(decoded);
        });
    });
}

fn bench_eoa_decode(c: &mut Criterion) {
    let codec = BcsCodec;
    let eoa_account = DbAccount(AccountInfo {
        balance: U256::ZERO,
        nonce: 42,
        code_hash: revm::primitives::KECCAK_EMPTY,
        code: None,
    });
    let encoded = codec.encode(&eoa_account);
    
    c.bench_function("eoa_account_decode_hot", |b| {
        b.iter(|| {
            let decoded: DbAccount = codec.try_decode(black_box(&encoded)).unwrap();
            black_box(decoded);
        });
    });
}

fn bench_contract_decode(c: &mut Criterion) {
    let codec = BcsCodec;
    let contract_account = DbAccount(AccountInfo {
        balance: U256::ZERO,
        nonce: 1,
        code_hash: B256::from_slice(&[0x12; 32]),
        code: None,
    });
    let encoded = codec.encode(&contract_account);
    
    c.bench_function("contract_account_decode_hot", |b| {
        b.iter(|| {
            let decoded: DbAccount = codec.try_decode(black_box(&encoded)).unwrap();
            black_box(decoded);
        });
    });
}

fn bench_cold_vs_hot(c: &mut Criterion) {
    let codec = BcsCodec;
    let mut group = c.benchmark_group("cold_vs_hot_comparison");
    
    // Test data
    let u64_val: u64 = 42;
    let u256_val = U256::from(12345678901234567890u64);
    let eoa_account = DbAccount(AccountInfo {
        balance: U256::ZERO,
        nonce: 42,
        code_hash: revm::primitives::KECCAK_EMPTY,
        code: None,
    });
    let contract_account = DbAccount(AccountInfo {
        balance: U256::ZERO,
        nonce: 1,
        code_hash: B256::from_slice(&[0x12; 32]),
        code: None,
    });
    
    // Pre-encode all data
    let u64_encoded = codec.encode(&u64_val);
    let u256_encoded = codec.encode(&u256_val);
    let eoa_encoded = codec.encode(&eoa_account);
    let contract_encoded = codec.encode(&contract_account);
    
    // Hot benchmarks (tight loops)
    group.bench_function("u64_hot_loop", |b| {
        b.iter(|| {
            let decoded: u64 = codec.try_decode(black_box(&u64_encoded)).unwrap();
            black_box(decoded);
        });
    });
    
    group.bench_function("u256_hot_loop", |b| {
        b.iter(|| {
            let decoded: U256 = codec.try_decode(black_box(&u256_encoded)).unwrap();
            black_box(decoded);
        });
    });
    
    group.bench_function("eoa_hot_loop", |b| {
        b.iter(|| {
            let decoded: DbAccount = codec.try_decode(black_box(&eoa_encoded)).unwrap();
            black_box(decoded);
        });
    });
    
    group.bench_function("contract_hot_loop", |b| {
        b.iter(|| {
            let decoded: DbAccount = codec.try_decode(black_box(&contract_encoded)).unwrap();
            black_box(decoded);
        });
    });
    
    // Cold simulation - create fresh codec and data each iteration
    group.bench_function("u64_cold_simulation", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let fresh_codec = BcsCodec;
                let fresh_data = fresh_codec.encode(&u64_val);
                let decoded: u64 = fresh_codec.try_decode(black_box(&fresh_data)).unwrap();
                black_box(decoded);
                // Add small delay to simulate real-world gaps between calls
                std::hint::black_box(std::thread::sleep(std::time::Duration::from_nanos(1)));
            }
            start.elapsed()
        });
    });
    
    group.bench_function("eoa_cold_simulation", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for _ in 0..iters {
                let fresh_codec = BcsCodec;
                let fresh_data = fresh_codec.encode(&eoa_account);
                let decoded: DbAccount = fresh_codec.try_decode(black_box(&fresh_data)).unwrap();
                black_box(decoded);
                // Add small delay to simulate real-world gaps between calls
                std::hint::black_box(std::thread::sleep(std::time::Duration::from_nanos(1)));
            }
            start.elapsed()
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_u64_decode,
    bench_u256_decode,
    bench_eoa_decode,
    bench_contract_decode,
    bench_cold_vs_hot
);
criterion_main!(benches);