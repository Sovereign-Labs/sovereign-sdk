mod metered_utils;
mod traits;

#[cfg(test)]
mod tests;

use std::marker::PhantomData;

pub use metered_utils::{
    metered_credential, MeteredBorshDeserialize, MeteredBorshDeserializeError, MeteredHasher,
    MeteredSigVerificationError, MeteredSignature,
};
pub use traits::*;

use crate::Spec;

/// A [`GasMeter`] that doesn't charge any gas.
#[derive(Clone, Default)]
pub struct UnlimitedGasMeter<S>(PhantomData<S>);

impl<S: Spec> GasMeter for UnlimitedGasMeter<S> {
    type Spec = S;
}
