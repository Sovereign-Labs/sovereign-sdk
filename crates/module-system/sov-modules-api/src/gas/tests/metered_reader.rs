use std::io::{self, Read};

use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::execution_mode::Native;
use sov_test_utils::storage::SimpleJmtStorageManager;
use sov_test_utils::MockDaSpec;

use crate::default_spec::DefaultSpec;
use crate::gas::tests::budget_for_reader_calls;
use crate::{
    Amount, Gas, GasMeteringError, GasPrice, GasUnit, MeteredReader, Spec, StateCheckpoint,
    WorkingSet,
};

type S = DefaultSpec<MockDaSpec, MockZkvm, MockZkvm, Native>;

const TEST_GAS_PRICE: GasPrice<2> = GasPrice {
    value: [Amount::new(1); 2],
};

fn create_working_set(remaining_funds: Amount) -> WorkingSet<S, StateCheckpoint<S>> {
    let storage_manager = SimpleJmtStorageManager::new();
    let storage = storage_manager.create_storage();
    WorkingSet::new_with_gas_meter(storage, remaining_funds, &TEST_GAS_PRICE)
}

fn budget_for_reads(per_byte: GasUnit<2>, per_read_bias: GasUnit<2>, reads: &[u32]) -> Amount {
    budget_for_reader_calls(per_byte, per_read_bias, reads).value(TEST_GAS_PRICE)
}

#[test]
fn read_exact_meters_against_slice() {
    // Regression test for the `<&[u8] as Read>::read_exact` bypass: that impl is a
    // direct memcpy that does NOT call `Read::read`. If `MeteredReader::read_exact`
    // is left as the default (which routes through `Read::read`), metering is silently
    // skipped against `&[u8]` sources. The test catches a missing override.
    let per_byte = GasUnit::<2>::from([10, 10]);
    let per_read_bias = GasUnit::<2>::from([5, 5]);

    // Budget exactly one read of 8 bytes. A second read of any size must run out of gas.
    let funds = budget_for_reads(per_byte, per_read_bias, &[8]);
    let mut ws = create_working_set(funds);

    let data: Vec<u8> = (0..16).collect();
    let mut reader =
        MeteredReader::new_with_prices(data.as_slice(), &mut ws, per_byte, per_read_bias);

    let mut buf = [0u8; 8];
    assert!(
        reader.read_exact(&mut buf).is_ok(),
        "first read_exact(8) must succeed"
    );
    assert_eq!(buf, [0, 1, 2, 3, 4, 5, 6, 7]);

    // Second read_exact must fail because the budget was exactly one read; the
    // returned io::Error must wrap a typed GasMeteringError.
    let mut buf2 = [0u8; 1];
    let err = reader
        .read_exact(&mut buf2)
        .expect_err("second read_exact must fail — gas is exhausted");
    assert!(
        err.downcast::<GasMeteringError<<S as Spec>::Gas>>().is_ok(),
        "out-of-gas io::Error must downcast to a typed GasMeteringError"
    );
}

#[test]
fn read_charges_per_byte_and_per_read_bias() {
    let per_byte = GasUnit::<2>::from([10, 10]);
    let per_read_bias = GasUnit::<2>::from([5, 5]);

    // Budget exactly one read of 4 bytes. Source has extra bytes so the second
    // read actually attempts a delivery (not EOF), which means it must fail on
    // the gas charge — that's what we're testing.
    let funds = budget_for_reads(per_byte, per_read_bias, &[4]);
    let mut ws = create_working_set(funds);

    let data = [9u8; 8];
    let mut reader =
        MeteredReader::new_with_prices(data.as_slice(), &mut ws, per_byte, per_read_bias);

    let mut buf = [0u8; 4];
    let n = reader.read(&mut buf).expect("first read must succeed");
    assert_eq!(n, 4);
    assert_eq!(buf, [9, 9, 9, 9]);

    let mut buf2 = [0u8; 1];
    assert!(
        reader.read(&mut buf2).is_err(),
        "no gas left — second read must fail at the charge step"
    );
}

#[test]
fn partial_read_charges_only_actual_bytes() {
    // A custom reader that delivers fewer bytes than requested. The metered reader
    // must charge only for the delivered bytes, not for the buffer size.
    struct ShortReader<'a> {
        data: &'a [u8],
        delivered: bool,
    }
    impl Read for ShortReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.delivered {
                return Ok(0);
            }
            self.delivered = true;
            // Always deliver only 3 bytes regardless of buf size.
            let n = self.data.len().min(buf.len()).min(3);
            buf[..n].copy_from_slice(&self.data[..n]);
            Ok(n)
        }
    }

    let per_byte = GasUnit::<2>::from([10, 10]);
    let per_read_bias = GasUnit::<2>::from([5, 5]);

    // Budget exactly 3 bytes worth of charge, not 10. If MeteredReader were charging
    // by buf size instead of actual bytes, this would fail.
    let funds = budget_for_reads(per_byte, per_read_bias, &[3]);
    let mut ws = create_working_set(funds);

    let data: Vec<u8> = (0..10).collect();
    let short = ShortReader {
        data: &data,
        delivered: false,
    };
    let mut reader = MeteredReader::new_with_prices(short, &mut ws, per_byte, per_read_bias);

    let mut buf = [0u8; 10];
    let n = reader
        .read(&mut buf)
        .expect("partial read must succeed within the 3-byte budget");
    assert_eq!(n, 3, "ShortReader always delivers 3 bytes");
}

#[test]
fn out_of_gas_during_read_wraps_typed_error() {
    let per_byte = GasUnit::<2>::from([10, 10]);
    let per_read_bias = GasUnit::<2>::from([5, 5]);

    // Budget zero gas — any read must fail at the per_read_bias charge.
    let mut ws = create_working_set(Amount::new(0));

    let data = [1u8; 16];
    let mut reader =
        MeteredReader::new_with_prices(data.as_slice(), &mut ws, per_byte, per_read_bias);

    let mut buf = [0u8; 4];
    let err = reader
        .read(&mut buf)
        .expect_err("read with zero gas budget must fail");
    assert!(
        err.downcast::<GasMeteringError<<S as Spec>::Gas>>().is_ok(),
        "out-of-gas io::Error must downcast to a typed GasMeteringError"
    );
}

#[test]
fn eof_read_does_not_charge() {
    let per_byte = GasUnit::<2>::from([10, 10]);
    let per_read_bias = GasUnit::<2>::from([5, 5]);

    // Empty input. Budget zero — if EOF charged, the read would fail.
    let mut ws = create_working_set(Amount::new(0));

    let data: &[u8] = &[];
    let mut reader = MeteredReader::new_with_prices(data, &mut ws, per_byte, per_read_bias);

    let mut buf = [0u8; 4];
    let n = reader
        .read(&mut buf)
        .expect("EOF (0 bytes returned) must not charge gas");
    assert_eq!(n, 0);
}
