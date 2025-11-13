//! Module call message definitions and runtime discriminant management.
//!
//! This module provides the core traits and types for handling module call messages
//! within the Sovereign SDK runtime system.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt::Debug;
use sov_universal_wallet::schema::UniversalWallet;

use crate::transaction::TransactionCallable;

/// A super trait bound representing all the traits required by [`super::Module::CallMessage`]s.
pub trait CallMessage:
    Debug
    + BorshSerialize
    + BorshDeserialize
    + UniversalWallet
    + schemars::JsonSchema
    + Clone
    + PartialEq
    + Eq
{
}

impl<
        T: Debug
            + BorshSerialize
            + BorshDeserialize
            + UniversalWallet
            + schemars::JsonSchema
            + Clone
            + PartialEq
            + Eq,
    > CallMessage for T
{
}

/// A trait that provides runtime discriminant information for module call messages.
///
/// Runtime discriminants are used to identify which module a call message belongs to
/// when deserializing from the RuntimeCall enum and serializing into a RuntimeCall compatible
/// message.
///
/// # Implementation Note
/// The discriminant should match the module's position in the runtime's module list
/// and the corresponding value in `constants.toml`.
/// This will eventually be automated.
pub trait RuntimeDiscriminant {
    /// Returns the runtime discriminant for this call message type.
    ///
    /// This value must match the module's discriminant defined in `constants.toml`
    /// and the module's position in the generated RuntimeCall enum.
    fn runtime_discriminant() -> u8;
}

/// A RuntimeCall implementation that operates independently of the Runtime itself.
///
/// This wrapper allows module call messages to be serialized and deserialized
/// without requiring access to the entire runtime or module instance. This is
/// particularly useful for:
///
/// - Client-side transaction building where the full runtime isn't available.
///   It makes call message (de)serialization deterministic.
/// - Testing scenarios where you want to serialize calls in isolation
/// - Closed-source module implementations that expose only their call interface
///
/// The discriminant is embedded directly in the serialized format, making
/// deserialization self-contained.
///
/// # Example
/// ```rust,ignore
/// // Define a call message with runtime discriminant
/// #[derive(/* all required traits */)]
/// struct MyModuleCall { /* fields */ }
///
/// impl RuntimeDiscriminant for MyModuleCall {
///     fn runtime_discriminant() -> u8 { 5 } // Module discriminant from constants.toml
/// }
///
/// // Use unmanaged wrapper for independent serialization
/// let call = UnmanagedRuntimeCall(MyModuleCall { /* data */ });
/// let serialized = borsh::to_vec(&call).unwrap();
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct UnmanagedRuntimeCall<T: RuntimeDiscriminant + CallMessage>(pub T);

impl<T: RuntimeDiscriminant + CallMessage> From<T> for UnmanagedRuntimeCall<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: RuntimeDiscriminant + CallMessage> BorshSerialize for UnmanagedRuntimeCall<T> {
    /// Serializes the call message with its runtime discriminant prefix.
    ///
    /// The serialization format is: [discriminant: u8][call_data: T]
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        T::runtime_discriminant().serialize(writer)?;
        self.0.serialize(writer)?;
        Ok(())
    }
}

impl<T: RuntimeDiscriminant + CallMessage> BorshDeserialize for UnmanagedRuntimeCall<T> {
    /// Deserializes the call message, validating the runtime discriminant.
    ///
    /// # Panics
    /// Panics if the discriminant in the serialized data doesn't match the
    /// expected discriminant for type `T`. This indicates either:
    /// - Corrupted data
    /// - Attempt to deserialize the wrong call type
    /// - Discriminant mismatch between sender and receiver
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let discriminant = u8::deserialize_reader(reader)?;
        let expected = T::runtime_discriminant();
        assert_eq!(
            discriminant,
            expected,
            "Runtime discriminant mismatch: found {}, expected {} for type {}",
            discriminant,
            expected,
            std::any::type_name::<T>()
        );
        Ok(Self(T::deserialize_reader(reader)?))
    }
}

impl<T: RuntimeDiscriminant + CallMessage> TransactionCallable for UnmanagedRuntimeCall<T> {
    type Call = T;
}
