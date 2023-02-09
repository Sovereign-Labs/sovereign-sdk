use sov_modules_api::Module;
mod example;

pub struct Transaction<C: sov_modules_api::Context> {
    pub message: _GenModuleEnumCall<C>,
    pub sender: C::PublicKey,
    pub signature: C::Signature,
}

// Generated
pub enum _GenModuleEnumCall<C: sov_modules_api::Context> {
    _Bank(<example::Bank<C> as Module>::CallMessage),
}

// Generated
pub enum _GenModuleEnumQuery<C: sov_modules_api::Context> {
    _Bank(<example::Bank<C> as Module>::QueryMessage),
}

// Generated
impl<C: sov_modules_api::Context> _GenModuleEnumCall<C> {
    pub fn dispatch_call(
        self,
        storage: C::Storage,
        context: C,
    ) -> Result<sov_modules_api::CallResponse, sov_modules_api::DecodingError> {
        match self {
            _GenModuleEnumCall::_Bank(call_msg) => {
                let mut bank = example::Bank::<C>::_new(storage);
                Ok(bank.call(call_msg, context)?)
            }
        }
    }
}

// Generated
impl<C: sov_modules_api::Context> _GenModuleEnumQuery<C> {
    pub fn dispatch_query(
        self,
        storage: C::Storage,
    ) -> Result<sov_modules_api::QueryResponse, sov_modules_api::DecodingError> {
        match self {
            _GenModuleEnumQuery::_Bank(query_msg) => {
                let bank = example::Bank::<C>::_new(storage);
                Ok(bank.query(query_msg)?)
            }
        }
    }
}
