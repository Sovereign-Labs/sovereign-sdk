#![no_main]

sp1_zkvm::entrypoint!(main);

use core::hint::black_box;
use std::io::{Cursor, Read};

use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::execution_mode::Zk;
use sov_modules_api::{MeteredBorshDeserialize, MeteredReader, UnlimitedGasMeter};

type MicrobenchSpec = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Zk>;

const MODE_READER_BYTES: u8 = 0;
const MODE_READER_COUNT: u8 = 1;
const MODE_DECODE_VEC: u8 = 2;

pub fn main() {
    let mode: u8 = sp1_zkvm::io::read();
    let iterations: u32 = sp1_zkvm::io::read();

    match mode {
        MODE_READER_BYTES => run_reader_bytes(iterations),
        MODE_READER_COUNT => run_reader_count(iterations),
        MODE_DECODE_VEC => run_decode_vec(iterations),
        other => panic!("unknown borsh bench mode {other}"),
    }
}

fn run_reader_bytes(iterations: u32) {
    let n_bytes: u32 = sp1_zkvm::io::read();
    let n = n_bytes as usize;

    let source: Vec<u8> = vec![0xABu8; n];
    let mut out: Vec<u8> = vec![0u8; n];
    let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();
    let source = black_box(source);
    let mut cursor = Cursor::new(source.as_slice());

    println!("cycle-tracker-report-start: borsh_loop");
    for _ in 0..iterations {
        cursor.set_position(0);
        let mut reader = MeteredReader::new(black_box(&mut cursor), &mut meter);
        reader
            .read_exact(black_box(&mut out))
            .expect("UnlimitedGasMeter never errors");
        let _ = black_box(&out);
    }
    println!("cycle-tracker-report-end: borsh_loop");

    sp1_zkvm::io::commit(&out);
}

fn run_reader_count(iterations: u32) {
    let n_reads: u32 = sp1_zkvm::io::read();

    let source: Vec<u8> = vec![0xABu8; n_reads as usize];
    let mut byte_buf = [0u8; 1];
    let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();
    let source = black_box(source);
    let mut cursor = Cursor::new(source.as_slice());

    println!("cycle-tracker-report-start: borsh_loop");
    for _ in 0..iterations {
        cursor.set_position(0);
        let mut reader = MeteredReader::new(black_box(&mut cursor), &mut meter);
        for _ in 0..n_reads {
            reader
                .read_exact(black_box(&mut byte_buf))
                .expect("UnlimitedGasMeter never errors");
            let _ = black_box(&byte_buf);
        }
    }
    println!("cycle-tracker-report-end: borsh_loop");

    sp1_zkvm::io::commit(&byte_buf);
}

fn run_decode_vec(iterations: u32) {
    let buf: Vec<u8> = sp1_zkvm::io::read_vec();
    let mut meter = UnlimitedGasMeter::<MicrobenchSpec>::default();
    let buf = black_box(buf);
    let mut last: Vec<u8> = Vec::new();

    println!("cycle-tracker-report-start: borsh_loop");
    for _ in 0..iterations {
        let mut slice: &[u8] = black_box(&buf[..]);
        last = <Vec<u8> as MeteredBorshDeserialize>::deserialize_from_slice(
            black_box(&mut slice),
            &mut meter,
        )
        .expect("UnlimitedGasMeter never errors");
        let _ = black_box(&last);
    }
    println!("cycle-tracker-report-end: borsh_loop");

    sp1_zkvm::io::commit(&last);
}
