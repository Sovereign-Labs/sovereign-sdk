use std::marker::PhantomData;

use crate::{GasMeter, Spec};

/// A [`GasMeter`] that doesn't charge any gas.
#[derive(Clone, Default)]
pub struct UnlimitedGasMeter<S>(PhantomData<S>);

impl<S: Spec> GasMeter for UnlimitedGasMeter<S> {
    type Spec = S;
}
