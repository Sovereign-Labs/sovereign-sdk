//! Utilities for working with higher-kinded types and traits.

/// A type which is generic over some type `M` and implements a useful trait bound
/// for all values of `M` that we care about (where "cases that we care about" depend on the specific trait).
///
/// A good example of this is the `DefaultSpec` type in  which is implemented for the [`Zk`](super::execution_mode::Zk) [`ExecutionMode`](super::execution_mode::ExecutionMode) in all cases
/// and for [`Native`](super::execution_mode::Native) and [`WitnessGeneration`](super::execution_mode::WitnessGeneration) modes when the `"native"` feature flag is enabled. We express
/// to the rust type system that `DefaultSpec` is implemented for all `ExecutionMode`s by implementing [`HigherKinded`] for it and generation
/// a higher-kinded trait `PluggableSpec` which allows us to switch between modes.
///
/// ```rust
/// # use sov_modules_api::higher_kinded_types::{Generic, HigherKindedHelper};
/// struct DefaultSpec<InnerZkvm, OuterZkvm, Mode>(std::marker::PhantomData<(InnerZkvm, OuterZkvm, Mode)>);
///
/// impl<InnerZkvm, OuterZkvm, Mode> Generic for DefaultSpec<InnerZkvm, OuterZkvm, Mode> {
///     type With<K> = DefaultSpec<InnerZkvm, OuterZkvm, K>;
/// }
///
/// impl<InnerZkvm, OuterZkvm, Mode> HigherKindedHelper for DefaultSpec<InnerZkvm, OuterZkvm, Mode> {
///     type Inner = Mode;
/// }
/// ```
pub trait HigherKinded: Sized + HigherKindedHelper {}

/// A helper trait for types which are [`HigherKinded`]. To implement that trait,
/// simply implement [`Generic`] and [`HigherKindedHelper`].
/// ```rust
/// # use sov_modules_api::higher_kinded_types::{Generic, HigherKindedHelper};
/// struct DefaultSpec<InnerZkvm, OuterZkvm, Mode>(std::marker::PhantomData<(InnerZkvm, OuterZkvm, Mode)>);
/// # impl<InnerZkvm, OuterZkvm, Mode> Generic for DefaultSpec<InnerZkvm, OuterZkvm, Mode> {
/// #     type With<K> = DefaultSpec<InnerZkvm, OuterZkvm, K>;
/// # }
///
/// impl<InnerZkvm, OuterZkvm, Mode> HigherKindedHelper for DefaultSpec<InnerZkvm, OuterZkvm, Mode> {
///     type Inner = Mode;
/// }
/// `````
pub trait HigherKindedHelper: Generic<With<Self::Inner> = Self> {
    /// The "inner" type that the higher kinded type is generic over.
    /// For example, `Spec` implementers are generic over an inner `ExecutionMode` type.
    type Inner;
}

impl<T> HigherKinded for T where T: Sized + HigherKindedHelper {}

/// A marker trait for generic structs where we want to be able to swap out a particular generic.
pub trait Generic {
    /// The type that is generic over `M`. This should be the type of `Self` but with one generic parameter set to `M`
    type With<M>;
}
