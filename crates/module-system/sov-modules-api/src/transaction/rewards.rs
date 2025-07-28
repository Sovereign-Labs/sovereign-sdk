use std::cmp::min;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::data::PriorityFeeBips;
use crate::{Amount, Gas, GasArray, Spec};

/// The format of the resources consumed by the transaction. The base fee and the priority fee are expressed as gas token amounts.
/// The [`TransactionConsumption`] data structure can only be built from the [`crate::WorkingSet`] data structure.
///
/// ## Type safety
/// To build this data structure outside of `sov-modules-api`, one would need to call [`crate::WorkingSet::finalize`]
#[derive(PartialEq, Eq, Debug, derive_more::Display, Serialize, Deserialize)]
#[display("{:?}", self)]
#[serde(bound = "GU: Serialize + serde::de::DeserializeOwned")]
pub struct TransactionConsumption<GU: Gas> {
    /// The amount of funds locked in the transaction that remains after transaction is executed and tip is processed.
    /// This amount includes the `base_fee` and the `priority_fee` gas token consumption
    pub(crate) remaining_funds: Amount,
    /// The base fee reward of the transaction expressed as a gas token amount.
    pub(crate) base_fee: GU,
    /// The priority fee reward of the transaction expressed as a gas token amount.
    pub(crate) priority_fee: Amount,
    /// The gas price of the transaction.
    pub(crate) gas_price: GU::Price,
}

impl<GU: Gas> TransactionConsumption<GU> {
    /// A zero consumption. Happens when the transaction is ignored (like in the case of a revert for the speculative execution mode).
    pub const ZERO: Self = Self {
        remaining_funds: Amount::ZERO,
        base_fee: GU::ZEROED,
        priority_fee: Amount::ZERO,
        gas_price: GU::Price::ZEROED,
    };

    /// Creates a new [`TransactionConsumption`] instance.
    pub fn new(
        remaining_funds: Amount,
        base_fee: GU,
        priority_fee: Amount,
        gas_price: GU::Price,
    ) -> Self {
        Self {
            remaining_funds,
            base_fee,
            priority_fee,
            gas_price,
        }
    }

    /// The base fee reward of the transaction expressed in multidimensional gas units.
    pub const fn base_fee(&self) -> &GU {
        &self.base_fee
    }

    /// The gas price used during the transaction.
    pub fn gas_price(&self) -> &GU::Price {
        &self.gas_price
    }

    /// The base fee reward of the transaction expressed as a gas token amount.
    /// This amounts to compute the scalar product of [`Self::base_fee`] by the current gas price.
    pub fn base_fee_value(&self) -> ProverReward {
        ProverReward(
            self.base_fee
                .checked_value(&self.gas_price)
                // SAFETY: `base_fee` comes from `BasicGasMeter`, which ensures overflow protection.
                .expect("Base fee value overflowed"),
        )
    }

    /// The priority fee reward of the transaction expressed as a gas token amount.
    pub const fn priority_fee(&self) -> SequencerReward {
        SequencerReward(self.priority_fee)
    }

    /// The remaining amount of gas tokens locked in the meter.
    pub fn remaining_funds(&self) -> RemainingFunds {
        RemainingFunds(self.remaining_funds)
    }
}

/// The prover reward.
#[derive(Copy, Debug, Clone, PartialEq, Eq, derive_more::Display)]
#[display("{:?}", self)]
pub struct ProverReward(pub Amount);

/// The remaining amount of gas tokens
#[derive(Copy, Debug, Clone, PartialEq, Eq, derive_more::Display)]
#[display("{:?}", self)]
pub struct RemainingFunds(pub Amount);

/// The type used to represent the sequencer reward. This type should be obtained from the [`TransactionConsumption`] type.
#[derive(
    Copy,
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    BorshSerialize,
    BorshDeserialize,
    derive_more::Into,
    derive_more::Display,
)]
#[display("SequencerReward({})", self.0)]
pub struct SequencerReward(pub Amount);

impl SequencerReward {
    /// Returns a zero sequencer reward. This can be used to initialize an accumulator to build a sequencer reward.
    pub const ZERO: Self = Self(Amount::ZERO);

    /// Adds another reward to this reward. Consumes the other reward.
    /// If the result overflows, we saturate.
    pub fn accumulate(&mut self, other: Self) {
        self.0 = self.0.saturating_add(other.0);
    }
}

/// Computes the transaction consumption for a given transaction.
/// This function is only used by the [`crate::WorkingSet`] to build a [`TransactionConsumption`] at the end of a transaction execution.
pub(crate) fn transaction_consumption_helper<S: Spec>(
    base_fee: &S::Gas,
    gas_price: &<S::Gas as Gas>::Price,
    max_fee: Amount,
    max_priority_fee_bips: PriorityFeeBips,
) -> TransactionConsumption<S::Gas> {
    let base_fee_value = base_fee
        .checked_value(gas_price)
        // SAFETY: `base_fee` comes from `BasicGasMeter`, which ensures overflow protection.
        .expect("Base fee value overflowed");

    let max_remaining_funds = max_fee.saturating_sub(base_fee_value);

    // We compute the `max_priority_fee_bips` by applying the `priority_fee_per_gas` to the consumed gas.
    let max_earned_priority_fee = max_priority_fee_bips
        .apply(base_fee_value)
        .unwrap_or(max_remaining_funds); // If the computation overflows, it would have been larger than the max_remaining_funds anyway - so just use that.

    // The tip is the minimum of the remaining gas allocated to the transaction and the maximum earned tip.
    // We transfer the tip to the tip recipient address.
    let priority_fee = min(max_earned_priority_fee, max_remaining_funds);

    // Since the tip is an amount of gas tokens consumed on top of the base fee from the gas meter, we need to take that into
    // account in the calculation.
    let remaining_funds = max_fee
        .saturating_sub(base_fee_value)
        .saturating_sub(priority_fee);

    TransactionConsumption {
        remaining_funds,
        base_fee: base_fee.clone(),
        priority_fee,
        gas_price: gas_price.clone(),
    }
}
