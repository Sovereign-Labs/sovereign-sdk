//! Implements "Pluggable" wrappers for traits needed by the rollup blueprint. These
//! wrappers encode the relationships between implementations of the traits for different
//! executione modes.
pub use spec::*;

mod spec {
    use sov_modules_api::higher_kinded_types::{Generic, HigherKinded};
    use sov_modules_api::Spec;

    /// An implementer of Runtime that can be used with any execution mode
    #[cfg(feature = "native")]
    pub trait PluggableSpec:
        HigherKinded
        + Spec
        + SpecImplementer<sov_rollup_interface::execution_mode::Native>
        + SpecImplementer<sov_rollup_interface::execution_mode::WitnessGeneration>
    {
    }

    #[cfg(feature = "native")]
    impl<T> PluggableSpec for T where
        T: HigherKinded
            + Spec
            + SpecImplementer<sov_rollup_interface::execution_mode::Native>
            + SpecImplementer<sov_rollup_interface::execution_mode::WitnessGeneration>
    {
    }

    /// An implementer of Runtime that can be used with any execution mode
    #[cfg(not(feature = "native"))]
    pub trait PluggableSpec:
        HigherKinded + Spec + SpecImplementer<sov_modules_api::execution_mode::Zk>
    {
    }

    #[cfg(not(feature = "native"))]
    impl<T> PluggableSpec for T where
        T: HigherKinded + Spec + SpecImplementer<sov_modules_api::execution_mode::Zk>
    {
    }

    /// A higher kinded type that implements `Spec` in some particular execution mode
    pub trait SpecImplementer<M>: Generic<With<M> = Self::SpecImpl> {
        /// The type that implements Runtime
        type SpecImpl: Spec;
    }

    impl<T, M> SpecImplementer<M> for T
    where
        T: Generic,
        <T as Generic>::With<M>: Spec,
    {
        type SpecImpl = <T as Generic>::With<M>;
    }
}
