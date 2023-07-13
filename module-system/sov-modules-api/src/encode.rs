use std::io::Read;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::NonInstantiable;

impl BorshDeserialize for NonInstantiable {
    fn deserialize_reader<R: Read>(_reader: &mut R) -> std::io::Result<Self> {
        unreachable!()
    }
}

impl BorshSerialize for NonInstantiable {
    fn serialize<W: std::io::Write>(&self, _writer: &mut W) -> std::io::Result<()> {
        unreachable!()
    }
}
