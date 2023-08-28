use sov_state::WorkingSet;

use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C> {
    pub fn end_slot_hook(&self, _root_hash: [u8; 32], _working_set: &mut WorkingSet<C::Storage>) {
        // TODO implement block creation logic.
    }
}
