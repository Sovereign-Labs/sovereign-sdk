use std::io::Read;

use borsh::{BorshDeserialize, BorshSerialize};
use revm::primitives::SpecId;

use crate::experimental::SpecIdWrapper;

impl BorshSerialize for SpecIdWrapper {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let value = self.0 as u8;
        value.serialize(writer)
    }
}

impl BorshDeserialize for SpecIdWrapper {
    fn deserialize(buf: &mut &[u8]) -> std::io::Result<Self> {
        let value = u8::deserialize(buf)?;
        Ok(SpecIdWrapper(SpecId::try_from_u8(value).unwrap()))
    }

    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let mut buf = vec![];
        reader.take(1).read_to_end(&mut buf).unwrap(); // Read 1 bytes for a u8
        let mut slice = buf.as_slice();
        SpecIdWrapper::deserialize(&mut slice)
    }
}
