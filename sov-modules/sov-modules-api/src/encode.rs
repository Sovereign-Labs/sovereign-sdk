use std::{convert::Infallible, io::Read};

use sovereign_sdk::serial::{Decode, DecodeBorrowed};

use crate::NonInstantiable;

impl<'de> DecodeBorrowed<'de> for NonInstantiable {
    type Error = Infallible;

    fn decode_from_slice(_: &'de [u8]) -> Result<Self, Self::Error> {
        unreachable!()
    }
}

impl Decode for NonInstantiable {
    type Error = Infallible;

    fn decode<R: Read>(_: &mut R) -> Result<Self, <Self as Decode>::Error> {
        unreachable!()
    }
}
