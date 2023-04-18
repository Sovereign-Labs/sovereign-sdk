use std::{convert::Infallible, io::Read};

use borsh::{BorshDeserialize, BorshSerialize};
use sovereign_sdk::serial::{Decode, DecodeBorrowed, Encode};

use crate::NonInstantiable;

impl BorshDeserialize for NonInstantiable {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        unreachable!()
    }
}

impl BorshSerialize for NonInstantiable {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        unreachable!()
    }
}
