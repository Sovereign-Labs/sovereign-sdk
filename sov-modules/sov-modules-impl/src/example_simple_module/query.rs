use super::ValueAdderModule;
use serde::{Deserialize, Serialize};
use sovereign_sdk::serial::{Decode, DecodeBorrowed};

pub enum QueryMessage {
    GetValue,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub struct Response {
    pub value: Option<u32>,
}

impl<C: sov_modules_api::Context> ValueAdderModule<C> {
    /// Queries the state of the module.
    pub fn query_value(&self) -> Response {
        Response {
            value: self.value.get(),
        }
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
