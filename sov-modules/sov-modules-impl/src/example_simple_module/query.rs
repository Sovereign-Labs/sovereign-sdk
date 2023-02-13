use super::ValueAdderModule;
use sov_modules_api::{QueryError, QueryResponse};
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

pub enum QueryMessage {
    GetValue,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub fn query_value(&self) -> Result<QueryResponse, QueryError> {
        let value = self.value.get();

        let response = serde_json::to_vec(&value).unwrap();
        Ok(QueryResponse { response })
    }
}

// Generated
impl<'de> DecodeBorrowed<'de> for QueryMessage {
    type Error = ();

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}

// Generated
impl Decode for QueryMessage {
    type Error = ();

    fn decode<R: std::io::Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        todo!()
    }
}
