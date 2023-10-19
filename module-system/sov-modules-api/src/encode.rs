use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::maybestd::io;

use crate::NonInstantiable;

impl BorshDeserialize for NonInstantiable {
    fn deserialize_reader<R: io::Read>(_reader: &mut R) -> io::Result<Self> {
        unreachable!()
    }
}

impl BorshSerialize for NonInstantiable {
    fn serialize<W: io::Write>(&self, _writer: &mut W) -> io::Result<()> {
        unreachable!()
    }
}
