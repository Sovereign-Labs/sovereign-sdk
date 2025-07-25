use avail_rust_core::H256;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHashTrait;
use std::convert::Infallible;
use std::fmt::{self, Display};
use std::io::{Read, Write};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AvailHash(pub H256);

impl Display for AvailHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl FromStr for AvailHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        H256::from_str(s)
            .map(Self)
            .map_err(|e| anyhow::anyhow!("invalid hash: {}", e))
    }
}

impl AsRef<[u8]> for AvailHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Into<[u8; 32]> for AvailHash {
    fn into(self) -> [u8; 32] {
        self.0.into()
    }
}

impl TryFrom<[u8; 32]> for AvailHash {
    type Error = Infallible;

    fn try_from(value: [u8; 32]) -> Result<Self, Self::Error> {
        Ok(Self(H256::from(value)))
    }
}

impl BorshSerialize for AvailHash {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(self.0.as_bytes())
    }
}

impl BorshDeserialize for AvailHash {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buf = [0u8; 32];
        reader.read_exact(&mut buf)?;
        Ok(AvailHash(H256::from(buf)))
    }
}

impl BlockHashTrait for AvailHash {}
