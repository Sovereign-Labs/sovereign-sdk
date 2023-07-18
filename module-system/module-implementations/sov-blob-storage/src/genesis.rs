use sov_state::WorkingSet;

use crate::BlobStorage;

impl<C: sov_modules_api::Context> BlobStorage<C> {
    pub(crate) fn init_module(
        &self,
        _config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> anyhow::Result<()> {
        self.blobs.set(&Vec::new(), working_set);
        Ok(())
    }
}
