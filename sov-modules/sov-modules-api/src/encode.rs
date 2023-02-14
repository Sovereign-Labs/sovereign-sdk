use std::io::Read;

use sovereign_sdk::serial::{Decode, DecodeBorrowed};

use crate::{DecodingError, NonInstantiable};

impl<'de> DecodeBorrowed<'de> for NonInstantiable {
    type Error = DecodingError;

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        unreachable!()
    }
}

impl Decode for NonInstantiable {
    type Error = DecodingError;

    fn decode<R: Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        unreachable!()
    }
}
