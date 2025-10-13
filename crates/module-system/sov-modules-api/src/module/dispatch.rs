//! Runtime call message definitions.

use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_universal_wallet::schema::UniversalWallet;
use strum::{VariantArray, VariantNames};

use super::ModuleInfo;
use crate::common::ModuleError;
use crate::module::{Context, Spec};
use crate::{ModuleId, StateProvider, WorkingSet};

/// A helper trait for working with enums like our generated `RuntimeCall` whose variants are tuples
/// containing a single field
pub trait NestedEnumUtils: VariantNames + AsRef<str> {
    /// An enum that consists of just the discriminant of the call message with no data.
    type Discriminants: VariantNames
        + VariantArray
        + Into<&'static str>
        + AsRef<str>
        + Clone
        + Copy
        + std::fmt::Debug;

    /// Returns the discriminant of the call message.
    fn discriminant(&self) -> Self::Discriminants;

    /// Returns the inner enum associated with this variant as a [`std::any::Any`]
    fn raw_contents(&self) -> &dyn std::any::Any;

    /// Returns the inner enum associated with this variant as a type-safe [`InnerEnumVariant`]
    fn contents(&self) -> InnerEnumVariant<'_> {
        InnerEnumVariant(self.raw_contents())
    }
}

/// The inner contents of nested enum.
pub struct InnerEnumVariant<'a>(&'a dyn std::any::Any);

impl<'a> InnerEnumVariant<'a> {
    /// Returns the contents of the nested enum.
    #[must_use]
    pub fn inner(&self) -> &dyn std::any::Any {
        self.0
    }

    /// A type-unsafe constructor for use in testing
    #[cfg(feature = "test-utils")]
    pub fn new_for_test(contents: &'a dyn std::any::Any) -> Self {
        Self(contents)
    }
}

/// A trait that needs to be implemented for any call message.
pub trait DispatchCall: Send + Sync {
    /// The context of the call
    type Spec: Spec;

    /// The concrete type that will decode into the call message of the module.
    type Decodable: Send
        + Sync
        + NestedEnumUtils
        + BorshSerialize
        + BorshDeserialize
        + Debug
        + PartialEq
        + Eq
        + Clone
        + UniversalWallet;

    /// Encode a [`Self::Decodable`]
    fn encode(decodable: &Self::Decodable) -> Vec<u8>;

    /// Dispatches a call message to the appropriate module.
    fn dispatch_call<I: StateProvider<Self::Spec>>(
        &mut self,
        message: Self::Decodable,
        state: &mut WorkingSet<Self::Spec, I>,
        context: &Context<Self::Spec>,
    ) -> Result<(), ModuleError>;

    /// Returns the ID of the dispatched module.
    fn module_id(&self, message: &Self::Decodable) -> &ModuleId;

    /// Returns the ID of the dispatched module.
    fn module_info(
        &self,
        discriminant: <Self::Decodable as NestedEnumUtils>::Discriminants,
    ) -> &dyn ModuleInfo<Spec = Self::Spec>;

    /// Validates that RuntimeCall enum discriminants match module discriminants from constants.toml
    /// Panics if there's a mismatch as this is not recoverable
    ///
    /// This check is currently useful when using the `UnmanagedRuntimeCall` type to add an extra
    /// check to ensure call message discriminants match. Note that this is still not fool proof,
    /// if the call messages RuntimeDiscriminant impl uses a completely different value
    /// then serialization can still fail when using `UnmanagedRuntimeCall`.
    ///
    /// The goal is to eventually do this automatically so there is no room for developer mistakes
    /// but it is currently tricky to derive RuntimeDiscriminant for call messages automatically.
    fn validate_discriminants(&self) {
        use strum::VariantArray;
        let runtime_variants =
            <<Self::Decodable as NestedEnumUtils>::Discriminants as VariantArray>::VARIANTS;

        for (enum_index, variant) in runtime_variants.iter().enumerate() {
            let variant_discriminant = self.module_info(*variant).discriminant();

            if enum_index as u8 != variant_discriminant {
                panic!(
                    "Discriminant mismatch for variant '{}': RuntimeCall enum position {} != module discriminant {}",
                    variant.as_ref(),
                    enum_index,
                    variant_discriminant
                );
            }
        }
    }
}
