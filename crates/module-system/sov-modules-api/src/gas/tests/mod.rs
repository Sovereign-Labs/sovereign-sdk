mod metered_reader;
mod metered_utils;

use crate::gas::GasArray;
use crate::GasUnit;

/// Gas cost of a sequence of reads through `MeteredReader`, each charging
/// `per_read_bias + per_byte × n`.
pub(super) fn budget_for_reader_calls(
    per_byte: GasUnit<2>,
    per_read_bias: GasUnit<2>,
    reads: &[u32],
) -> GasUnit<2> {
    let mut total = GasUnit::<2>::ZEROED;
    for &n in reads {
        total = total
            .checked_combine(per_read_bias)
            .unwrap()
            .checked_combine(per_byte.checked_scalar_product(u64::from(n)).unwrap())
            .unwrap();
    }
    total
}
