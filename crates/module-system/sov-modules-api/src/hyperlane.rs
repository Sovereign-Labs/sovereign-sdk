use sov_rollup_interface::common::{HexHash, HexString};

use crate::{Address, Base58Address};

/// An address which is compatible with the hyperlane protocol.
///
/// Implementers of this trait must ensure that their addresses can be unambiguously represented in 32 bytes.
/// For example, if the address type is an enum where at least one variant is 32 bytes long, the impelementation
/// must pick one variant and always deserialize into that type (since there's no room to encode the discriminant).
pub trait HyperlaneAddress: Sized {
    /// Convert the address to a Hyperlane sender address.
    fn to_sender(&self) -> HexHash;
    /// Convert a Hyperlane sender address back to the original.
    fn from_sender(recipient: HexHash) -> anyhow::Result<Self>;
}

impl HyperlaneAddress for Address {
    fn to_sender(&self) -> HexHash {
        const START_INDEX: usize = 32 - Address::LENGTH;
        // Pad the address with leading zeros to 32 bytes. This is the hyperlane convention
        let mut bytes = [0u8; 32];
        bytes[START_INDEX..].copy_from_slice(self.as_ref());
        bytes.into()
    }

    fn from_sender(recipient: HexHash) -> anyhow::Result<Self> {
        const START_INDEX: usize = 32 - Address::LENGTH;
        // Check that the address is padded with leading zeros to match the hyperlane convention.
        let (padding, address) = recipient.0.split_at(START_INDEX);

        // Ensure padding is all zeros:
        anyhow::ensure!(
            padding.iter().all(|&byte| byte == 0),
            "Invalid address - not enough leading zeros"
        );

        Ok(Self::new(address.try_into().expect(
            "Infallible conversion failed; this is a bug, please report it",
        )))
    }
}

impl HyperlaneAddress for Base58Address {
    /// Convert the address to a Hyperlane sender address.
    fn to_sender(&self) -> HexHash {
        HexString(self.0)
    }
    /// Convert a Hyperlane sender address back to the original..
    fn from_sender(recipient: HexHash) -> anyhow::Result<Self> {
        Ok(Self(recipient.0))
    }
}
