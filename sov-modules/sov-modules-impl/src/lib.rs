use sov_modules_api::Module;
mod example_simple;

pub struct Transaction<C: sov_modules_api::Context> {
    pub message: _GenModuleEnumCall<C>,
    pub sender: C::PublicKey,
    pub signature: C::Signature,
}

// Generated
pub enum _GenModuleEnumCall<C: sov_modules_api::Context> {
    _Bank(<example_simple::ValueAdderModule<C> as Module>::CallMessage),
}

// Generated
pub enum _GenModuleEnumQuery<C: sov_modules_api::Context> {
    _Bank(<example_simple::ValueAdderModule<C> as Module>::QueryMessage),
}

// Generated
impl<C: sov_modules_api::Context> _GenModuleEnumCall<C> {
    pub fn dispatch_call(
        self,
        storage: C::Storage,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, sov_modules_api::CallError> {
        match self {
            _GenModuleEnumCall::_Bank(call_msg) => {
                let mut bank = example_simple::ValueAdderModule::<C>::_new(storage);
                bank.call(call_msg, context)
            }
        }
    }
}

// Generated
impl<C: sov_modules_api::Context> _GenModuleEnumQuery<C> {
    pub fn dispatch_query(
        self,
        storage: C::Storage,
    ) -> Result<sov_modules_api::QueryResponse, sov_modules_api::QueryError> {
        match self {
            _GenModuleEnumQuery::_Bank(query_msg) => {
                let bank = example_simple::ValueAdderModule::<C>::_new(storage);
                bank.query(query_msg)
            }
        }
    }
}
