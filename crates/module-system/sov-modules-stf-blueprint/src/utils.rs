use sov_modules_api::Spec;

use crate::{Runtime, StfBlueprint};

impl<S: Spec, RT: Runtime<S>> StfBlueprint<S, RT> {
    /// Returns the underlying runtime.
    pub fn runtime(&self) -> &RT {
        &self.runtime
    }
}
