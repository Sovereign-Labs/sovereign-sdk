use super::{Bank, Delete, Transfer};

impl<C: sov_modules_api::Context> Bank<C> {
    pub(crate) fn do_transfer(
        &mut self,
        transfer: Transfer<C>,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, <Self as sov_modules_api::Module>::CallError> {
        if transfer.from != context.sender() {
            todo!()
        }

        Ok(sov_modules_api::CallResponse::default())
    }

    pub(crate) fn do_delete(
        &self,
        delete: Delete<C>,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, <Self as sov_modules_api::Module>::CallError> {
        if delete.id != context.sender() {
            todo!()
        }

        Ok(sov_modules_api::CallResponse::default())
    }
}
