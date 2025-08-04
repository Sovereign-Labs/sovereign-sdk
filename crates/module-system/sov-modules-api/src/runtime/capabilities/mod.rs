#![deny(missing_docs)]
//! The rollup capabilities module defines "capabilities" that rollup must
//! provide if they wish to use the standard app template.
//! If you don't want to provide these capabilities,
//! you can bypass the Sovereign module-system completely
//! and write a state transition function from scratch.
//! [See here for docs](https://github.com/Sovereign-Labs/sovereign-sdk/blob/nightly/examples/demo-stf/README.md)
pub mod authentication;
pub mod authorization;
mod batch_selector;
mod kernel;
mod proof;

#[cfg(feature = "native")]
use std::sync::Arc;

pub use authentication::*;
pub use authorization::*;
pub use batch_selector::*;
pub use kernel::*;

mod gas;
pub use gas::*;
pub use proof::ProofProcessor;
mod sequencer;
pub use sequencer::*;
mod chain_state;
pub use chain_state::*;
pub use sov_rollup_interface::common::RollupHeight;

use crate::Spec;

/// Wrapper around an inner type that prevents accessing it.
pub struct Guard<T> {
    inner: T,
}

impl<T> Guard<T> {
    /// Create a new guarded instance.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

/// Indicates that a type provides the necessary capabilities for a runtime.
///
/// Capabilities are important core functionality like charging for gas, authorizing transactions,
/// and processing proofs. While overriding these capabilities is allowed and can be very useful, it is also dangerous -
/// using a non-standard implementation can cause subtle bugs. Some capabilities are coupled with one another - for example,
/// the sequencer registry sets asides funds that are used to reward the prover later on.
///
/// Take great care when overriding these capabilities. Always test your changes thoroughly, and be sure that you understand
/// the implications of your changes.
///
/// If you just want a sensible default implementation, use the `StandardCapabilities` struct provided by the SDK maintainers.
pub trait HasCapabilities<S: Spec> {
    /// The concrete implementation of the capabilities.
    type Capabilities<'a>: GasEnforcer<S>
        + SequencerAuthorization<S>
        + TransactionAuthorizer<S>
        + ProofProcessor<S>
        + SequencerRemuneration<S>
    where
        Self: 'a;

    /// Fetches the capabilities from the runtime.
    ///
    /// This method is only intended to be used internally on the [`HasCapabilities`] trait, if you
    /// need to access a capability do so with the constructor method.
    ///
    /// The returned struct is wrapped in a guard to prevent access from code outside of the trait.
    /// Without the guard it would be possible to implement an override for a capability and
    /// accidently use the default implementation leading subtle type mismatch bugs.
    ///
    /// For example, if I override [`HasCapabilities::gas_enforcer`] to return a different [`GasEnforcer`]
    /// implementation but then used `HasCapabilities::capabilities().try_reserve_gas` instead of
    /// `HasCapabilities::gas_enforcer().try_reserve_gas` I would use the default implementation instead of
    /// the override.
    fn capabilities(&mut self) -> Guard<Self::Capabilities<'_>>;

    /// Returns the [`GasEnforcer`] implementation on [`HasCapabilities::Capabilities`].
    ///
    /// This method can be overriden to provide a custom implementation.
    fn gas_enforcer(&mut self) -> impl GasEnforcer<S> {
        self.capabilities().inner
    }

    /// Returns the [`SequencerAuthorization`] implementation on [`HasCapabilities::Capabilities`].
    ///
    /// This method can be overriden to provide a custom implementation.
    fn sequencer_authorization(&mut self) -> impl SequencerAuthorization<S> {
        self.capabilities().inner
    }

    /// Returns the [`TransactionAuthorizer`] implementation on [`HasCapabilities::Capabilities`].
    ///
    /// This method can be overriden to provide a custom implementation.
    fn transaction_authorizer(&mut self) -> impl TransactionAuthorizer<S> {
        self.capabilities().inner
    }

    /// Returns the [`ProofProcessor`] implementation on [`HasCapabilities::Capabilities`].
    ///
    /// This method can be overriden to provide a custom implementation.
    fn proof_processor(&mut self) -> impl ProofProcessor<S> {
        self.capabilities().inner
    }

    /// Returns the [`SequencerRemuneration`] implementation on [`HasCapabilities::Capabilities`].
    ///
    /// This method can be overriden to provide a custom implementation.
    fn sequencer_remuneration(&mut self) -> impl SequencerRemuneration<S> {
        self.capabilities().inner
    }
}

/// Indicates that a type provides the necessary kernel capabilities for a runtime.
pub trait HasKernel<S: Spec>: Default + Send + Sync + 'static {
    /// The concrete implementation of the kernel.
    type Kernel<'a>: Kernel<S> + ChainState<Spec = S> + BlobSelector<Spec = S>
    where
        Self: 'a;

    /// Fetches the kernel modules from the runtime.
    ///
    /// This method is only intended to be used internally on the [`HasKernel`] trait, if you
    /// need to access a kernel capability do so with the constructor method.
    ///
    /// The returned struct is wrapped in a guard to prevent access from code outside of the trait.
    /// Without the guard it would be possible to implement an override for a kernel capability and
    /// accidently use the default implementation leading subtle type mismatch bugs.
    ///
    /// For example, if I override [`HasCapabilities::gas_enforcer`] to return a different [`GasEnforcer`]
    /// implementation but then used `HasCapabilities::capabilities().try_reserve_gas` instead of
    /// `HasCapabilities::gas_enforcer().try_reserve_gas` I would use the default implementation instead of
    /// the override.
    fn inner(&mut self) -> Guard<Self::Kernel<'_>>;

    /// Returns the [`Kernel`] implementation on [`HasKernel::Kernel`].
    fn kernel(&mut self) -> Self::Kernel<'_> {
        self.inner().inner
    }

    /// Returns the [`ChainState`] implementation on [`HasKernel::Kernel`].
    fn chain_state(&mut self) -> impl ChainState<Spec = S> {
        self.inner().inner
    }

    /// Returns the [`BlobSelector`] implementation on [`HasKernel::Kernel`].
    fn blob_selector(&mut self) -> impl BlobSelector<Spec = S> {
        self.inner().inner
    }

    /// Returns the [`KernelWithSlotMapping`] implementation on [`HasKernel::Kernel`].
    #[cfg(feature = "native")]
    fn kernel_with_slot_mapping(&self) -> Arc<dyn KernelWithSlotMapping<S>>;
}

#[cfg(feature = "test-utils")]
pub mod mocks {
    //! Mocks for the rollup capabilities module

    use sov_rollup_interface::common::{SlotNumber, VisibleSlotNumber};

    #[cfg(feature = "native")]
    use super::KernelWithSlotMapping;
    use super::{Kernel, RollupHeight, Spec};
    use crate::BootstrapWorkingSet;
    #[cfg(feature = "native")]
    use crate::GetGasPrice;

    /// A mock kernel for use in tests
    #[derive(Debug, Clone, Default)]
    pub struct MockKernel<S> {
        /// The current slot number
        pub true_slot_number: SlotNumber,
        /// The slot number at which transactions appear to be executing
        pub visible_slot_number: VisibleSlotNumber,
        /// The next sequence number to expect for preferred blobs.
        pub next_sequence_number: u64,
        phantom: core::marker::PhantomData<S>,
    }

    impl<S: Spec> MockKernel<S> {
        /// Create a new mock kernel with the given slot numbers
        pub fn new(true_slot_number: u64, visible_slot_number: u64) -> Self {
            Self {
                true_slot_number: SlotNumber::new_dangerous(true_slot_number),
                visible_slot_number: VisibleSlotNumber::new_dangerous(visible_slot_number),
                next_sequence_number: 0,
                phantom: core::marker::PhantomData,
            }
        }

        /// Simply increases all the heights by one
        pub fn increase_heights(&mut self) {
            self.true_slot_number.incr();
            self.visible_slot_number.incr();
        }
    }

    #[cfg(feature = "native")]
    impl<S: Spec> KernelWithSlotMapping<S> for MockKernel<S> {
        fn visible_slot_number_at(
            &self,
            true_slot_number: SlotNumber,
            _state: &mut crate::ApiStateAccessor<S>,
        ) -> Option<VisibleSlotNumber> {
            Some(true_slot_number.as_visible())
        }

        fn current_rollup_height(
            &self,
            _state: &mut crate::state::ApiStateAccessor<S>,
        ) -> RollupHeight {
            RollupHeight::new(self.visible_slot_number.get())
        }

        fn rollup_height_to_visible_slot_number(
            &self,
            height: super::RollupHeight,
            _state: &mut crate::state::ApiStateAccessor<S>,
        ) -> Option<VisibleSlotNumber> {
            Some(VisibleSlotNumber::new_dangerous(height.get()))
        }

        #[cfg(feature = "native")]
        fn true_slot_number_at_historical_height(
            &self,
            height: super::RollupHeight,
            _state: &mut crate::state::ApiStateAccessor<S>,
        ) -> Option<SlotNumber> {
            Some(SlotNumber::new_dangerous(height.get()))
        }

        fn base_fee_per_gas_at(
            &self,
            _height: super::RollupHeight,
            state: &mut crate::state::ApiStateAccessor<S>,
        ) -> Option<<<S as Spec>::Gas as crate::Gas>::Price> {
            Some(state.gas_price().clone())
        }

        fn true_slot_number_to_rollup_height(
            &self,
            _true_slot_number: SlotNumber,
            _state: &mut crate::state::ApiStateAccessor<S>,
        ) -> Option<RollupHeight> {
            Some(RollupHeight::new(self.visible_slot_number.get()))
        }
    }

    impl<S: Spec> Kernel<S> for MockKernel<S> {
        fn true_slot_number(&self, _ws: &mut BootstrapWorkingSet<'_, S>) -> SlotNumber {
            self.true_slot_number
        }
        fn next_visible_slot_number(
            &self,
            _ws: &mut BootstrapWorkingSet<'_, S>,
        ) -> VisibleSlotNumber {
            self.visible_slot_number
        }

        fn rollup_height(&self, _state: &mut BootstrapWorkingSet<'_, S>) -> super::RollupHeight {
            RollupHeight::new(self.visible_slot_number.get())
        }

        fn record_gas_usage(
            &mut self,
            _state: &mut crate::StateCheckpoint<S>,
            _final_gas_info: super::BlockGasInfo<<S as Spec>::Gas>,
            _rollup_height: RollupHeight,
        ) {
        }
    }
}
