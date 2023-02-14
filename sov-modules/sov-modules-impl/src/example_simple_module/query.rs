use super::ValueAdderModule;
use serde::{Deserialize, Serialize};
use sov_modules_api::QueryResponse;
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

pub enum QueryMessage {
    GetValue,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    pub fn query_value(&self) -> QueryResponse {
        let response = Response {
            value: self.value.get(),
        };

        let response = serde_json::to_vec(&response).unwrap();
        QueryResponse { response }
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
