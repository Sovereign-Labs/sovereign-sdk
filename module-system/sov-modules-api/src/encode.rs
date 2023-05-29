use crate::NonInstantiable;
use borsh::{BorshDeserialize, BorshSerialize};
use std::io::Read;

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
