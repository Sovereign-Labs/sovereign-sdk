#![deny(missing_docs)]
//! The rollup capabilities module defines "capabilities" that rollup must
//! provide if they wish to use the standard app template.
//! If you don't want to provide these capabilities,
//! you can bypass the Sovereign module-system completely
//! and write a state transition function from scratch.
//! [See here for docs](https://github.com/Sovereign-Labs/sovereign-sdk/blob/nightly/examples/demo-stf/README.md)

use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};

use crate::{Context, KernelWorkingSet, Spec, Storage, WorkingSet};

/// The kernel is responsible for managing the inputs to the `apply_blob` method.
/// A simple implementation will simply process all blobs in the order that they appear,
/// while a second will support a "preferred sequencer" with some limited power to reorder blobs
/// in order to give out soft confirmations.
pub trait Kernel<C: Context, Da: DaSpec>: BlobSelector<Da, Context = C> + Default {
    /// GenesisConfig type.
    type GenesisConfig: Send + Sync;

    #[cfg(feature = "native")]
    /// GenesisPaths type.
    type GenesisPaths: Send + Sync;

    /// Initialize the kernel at genesis
    fn genesis(
        &self,
        config: &Self::GenesisConfig,
        working_set: &mut WorkingSet<C>,
    ) -> Result<(), anyhow::Error>;

    /// Return the current slot height
    fn true_height(&self, working_set: &mut WorkingSet<C>) -> u64;
    /// Return the height at which transactions currently *appear* to be executing.
    fn visible_height(&self, working_set: &mut WorkingSet<C>) -> u64;
}

/// Hooks allowing the kernel to get access to the DA layer state
pub trait KernelSlotHooks<C: Context, Da: DaSpec>: Kernel<C, Da> {
    /// Called at the beginning of a slot
    fn begin_slot_hook(
        &self,
        slot_header: &Da::BlockHeader,
        validity_condition: &Da::ValidityCondition,
        pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        working_set: &mut WorkingSet<Self::Context>,
    );
    /// Called at the end of a slot
    fn end_slot_hook(&self, working_set: &mut WorkingSet<Self::Context>);
}

/// BlobSelector decides which blobs to process in a current slot.
pub trait BlobSelector<Da: DaSpec> {
    /// Context type
    type Context: Context;

    /// It takes two arguments.
    /// 1. `current_blobs` - blobs that were received from the network for the current slot.
    /// 2. `working_set` - the working to access storage.
    /// It returns a vector containing a mix of borrowed and owned blobs.
    fn get_blobs_for_this_slot<'a, 'k, I>(
        &self,
        current_blobs: I,
        working_set: &mut KernelWorkingSet<'k, Self::Context>,
    ) -> anyhow::Result<alloc::vec::Vec<BlobRefOrOwned<'a, Da::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>;
}

/// Container type for mixing borrowed and owned blobs.
#[derive(Debug)]
pub enum BlobRefOrOwned<'a, B: BlobReaderTrait> {
    /// Mutable reference
    Ref(&'a mut B),
    /// Owned blob
    Owned(B),
}

impl<'a, B: BlobReaderTrait> AsRef<B> for BlobRefOrOwned<'a, B> {
    fn as_ref(&self) -> &B {
        match self {
            BlobRefOrOwned::Ref(r) => r,
            BlobRefOrOwned::Owned(blob) => blob,
        }
    }
}

impl<'a, B: BlobReaderTrait> BlobRefOrOwned<'a, B> {
    /// Convenience method to get mutable reference to the blob
    pub fn as_mut_ref(&mut self) -> &mut B {
        match self {
            BlobRefOrOwned::Ref(r) => r,
            BlobRefOrOwned::Owned(ref mut blob) => blob,
        }
    }
}

impl<'a, B: BlobReaderTrait> From<B> for BlobRefOrOwned<'a, B> {
    fn from(value: B) -> Self {
        BlobRefOrOwned::Owned(value)
    }
}

impl<'a, B: BlobReaderTrait> From<&'a mut B> for BlobRefOrOwned<'a, B> {
    fn from(value: &'a mut B) -> Self {
        BlobRefOrOwned::Ref(value)
    }
}

#[cfg(feature = "mocks")]
pub mod mocks {
    //! Mocks for the rollup capabilities module

    use sov_rollup_interface::da::DaSpec;

    use super::{BlobRefOrOwned, BlobSelector, Kernel};
    use crate::{Context, WorkingSet};

    /// A mock kernel for use in tests
    #[derive(Debug, Clone)]
    pub struct MockKernel<C, Da> {
        /// The current slot height
        pub true_height: u64,
        /// The height at which transactions appear to be executing
        pub visible_height: u64,
        phantom: core::marker::PhantomData<(C, Da)>,
    }

    impl<C, Da> Default for MockKernel<C, Da> {
        fn default() -> Self {
            Self {
                true_height: 0,
                visible_height: 0,
                phantom: Default::default(),
            }
        }
    }

    impl<C: Context, Da: DaSpec> MockKernel<C, Da> {
        /// Create a new mock kernel with the given slot height
        pub fn new(true_height: u64, visible_height: u64) -> Self {
            Self {
                true_height,
                visible_height,
                phantom: core::marker::PhantomData,
            }
        }
    }

    impl<C: Context, Da: DaSpec> Kernel<C, Da> for MockKernel<C, Da> {
        fn true_height(&self, _ws: &mut WorkingSet<C>) -> u64 {
            self.true_height
        }
        fn visible_height(&self, _ws: &mut WorkingSet<C>) -> u64 {
            self.visible_height
        }

        type GenesisConfig = ();

        #[cfg(feature = "native")]
        type GenesisPaths = ();

        fn genesis(
            &self,
            _config: &Self::GenesisConfig,
            _working_set: &mut WorkingSet<C>,
        ) -> Result<(), anyhow::Error> {
            Ok(())
        }
    }

    impl<C: Context, Da: DaSpec> BlobSelector<Da> for MockKernel<C, Da> {
        type Context = C;

        fn get_blobs_for_this_slot<'a, 'k, I>(
            &self,
            current_blobs: I,
            _working_set: &mut crate::KernelWorkingSet<'k, Self::Context>,
        ) -> anyhow::Result<alloc::vec::Vec<super::BlobRefOrOwned<'a, Da::BlobTransaction>>>
        where
            I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
        {
            Ok(current_blobs.into_iter().map(BlobRefOrOwned::Ref).collect())
        }
    }
}
