use sov_modules_api::macros::serialize;
use sov_modules_api::{Amount, DaSpec, Spec};

/// Sample Event
#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(bound = "S: Spec", rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
pub enum Event<S: Spec> {
    /// A sequencer was registered.
    Registered {
        /// The address of the sequencer that was registered.
        sequencer: S::Address,
        /// The amount of the initial deposit.
        amount: Amount,
    },

    /// A sequencer initiated a withdrawal.
    InitiatedWithdrawal {
        /// The address of the sequencer that initiated the withdrawal.
        sequencer: S::Address,
    },

    /// A sequencer exited.
    Withdrew {
        /// The address of the sequencer that exited.
        sequencer: S::Address,
        /// The amount that was withdrawn.
        amount_withdrawn: Amount,
    },

    /// A sequencer deposited funds to stake.
    Deposited {
        /// The address of the sequencer that was deposited to.
        sequencer: S::Address,
        /// The amount of the deposit.
        amount: u128,
    },

    /// A sequencer rotated its DA address.
    ///
    /// Preferred self-rotations emit this when the rotation is scheduled; the
    /// state move is applied later in the end-block hook.
    DaAddressUpdated {
        /// The rollup address of the sequencer (unchanged across the rotation).
        sequencer: S::Address,
        /// The DA address being rotated away from.
        old_da_address: <S::Da as DaSpec>::Address,
        /// The new DA address.
        new_da_address: <S::Da as DaSpec>::Address,
    },
}
