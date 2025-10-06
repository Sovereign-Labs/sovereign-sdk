use thiserror::Error;

use crate::{Amount, Gas};

/// Error type that can be raised by the `GasMeter` trait.
/// Errors can be raised either when the meter runs out of gas or when the refund operation fails.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GasMeteringError<GU: Gas> {
    #[error("Gas calculation overflow: {0}")]
    /// Unable to calculate gas usage due to overflow.
    Overflow(String),
    /// The slot gas limit has been exhausted.
    #[error("The slot gas limit has been exhausted. Initial slot gas: {initial_slot_gas}, gas to charge {gas_to_charge}, remaining total slot gas {remaining_total_slot_gas}, remaining preferred slot gas {remaining_preferred_slot_gas}, is preffred {is_preferred}")]
    SlotOutOfGas {
        /// The initial slot gas limit.
        initial_slot_gas: GU,
        /// The amount of gas to charge.
        gas_to_charge: GU,
        /// The remaining preffered gas.
        remaining_preferred_slot_gas: GU,
        /// The remaining total slot gas.
        remaining_total_slot_gas: GU,
        /// Gas allocated to transactions from preferred sequencer.
        is_preferred: bool,
    },
    /// Unable to deserialize data due to invalid length.
    #[error("Unable to deserialize data due to invalid length: {0}")]
    InvalidLength(String),
    /// The gas meter has ran out of gas.
    #[error("The gas to charge is greater than the funds available in the meter. Gas to charge {gas_to_charge}, gas price {gas_price}, initial_gas {initial_gas}, remaining gas {remaining_gas}")]
    OutOfGas {
        /// The amount of gas to charge.
        gas_to_charge: GU,
        /// The current gas price.
        gas_price: GU::Price,
        /// The initial gas.
        initial_gas: GU,
        /// The remaining gas.
        remaining_gas: GU,
    },
    /// The gas meter has ran out of funds.
    #[error("The amount to charge is greater than the funds available in the meter. Amount to charge {amount_to_charge}, remaining_funds  {remaining_funds}, price {gas_price}")]
    OutOfFunds {
        /// The amount to charge.
        amount_to_charge: Amount,
        /// Remaining funds.
        remaining_funds: Amount,
        /// The current gas price.
        gas_price: GU::Price,
    },
    /// The refund operation failed for the gas meter.
    #[error("The gas to refund is greater than the gas used. Gas to refund {gas_to_refund}, gas used {gas_used}")]
    ImpossibleToRefundGas {
        /// Amount of gas to refund.
        gas_to_refund: GU,
        /// Amount of gas currently used by the meter.
        gas_used: GU,
    },
}
