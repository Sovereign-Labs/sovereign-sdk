use super::ValueAdderModule;
use sov_modules_api::{QueryError, QueryResponse};

pub enum QueryMessage {
    GetValue,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub fn query_value(&self) -> Result<QueryResponse, QueryError> {
        let value = self.value.get();

        let data = serde_json::to_vec(&value).unwrap();

        Ok(QueryResponse { response: data })
    }
}
