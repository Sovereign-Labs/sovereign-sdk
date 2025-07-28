//! Implements Hyperlane's incremental merkle tree.
//!
//! This is translated from the implementation from cw-hyperlane:
//! <https://github.com/many-things/cw-hyperlane/blob/7573576c97fe9ee9a91c3e4557ff5a32bfbcee40/packages/interface/src/types/merkle.rs#L11>

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sov_modules_api::{GasMeter, HexHash, HexString, Spec};

use crate::crypto::keccak256_concat;

pub const TREE_DEPTH: usize = 32;
pub const ZERO_BYTES: HexHash = HexString([0u8; 32]);
// See <https://github.com/eigerco/hyperlane-monorepo/blob/cb6727f013e82884e15966edd863e3a888fa9184/solidity/contracts/libs/Merkle.sol#L173>
// You can find a nice hex-encoded list of the zeroes in the tests below
pub const ZERO_HASHES: [HexHash; TREE_DEPTH] = [
    ZERO_BYTES,
    HexString([
        173, 50, 40, 182, 118, 247, 211, 205, 66, 132, 165, 68, 63, 23, 241, 150, 43, 54, 228, 145,
        179, 10, 64, 178, 64, 88, 73, 229, 151, 186, 95, 181,
    ]),
    HexString([
        180, 193, 25, 81, 149, 124, 111, 143, 100, 44, 74, 246, 28, 214, 178, 70, 64, 254, 198,
        220, 127, 198, 7, 238, 130, 6, 169, 158, 146, 65, 13, 48,
    ]),
    HexString([
        33, 221, 185, 163, 86, 129, 92, 63, 172, 16, 38, 182, 222, 197, 223, 49, 36, 175, 186, 219,
        72, 92, 155, 165, 163, 227, 57, 138, 4, 183, 186, 133,
    ]),
    HexString([
        229, 135, 105, 179, 42, 27, 234, 241, 234, 39, 55, 90, 68, 9, 90, 13, 31, 182, 100, 206,
        45, 211, 88, 231, 252, 191, 183, 140, 38, 161, 147, 68,
    ]),
    HexString([
        14, 176, 30, 191, 201, 237, 39, 80, 12, 212, 223, 201, 121, 39, 45, 31, 9, 19, 204, 159,
        102, 84, 13, 126, 128, 5, 129, 17, 9, 225, 207, 45,
    ]),
    HexString([
        136, 124, 34, 189, 135, 80, 211, 64, 22, 172, 60, 102, 181, 255, 16, 45, 172, 221, 115,
        246, 176, 20, 231, 16, 181, 30, 128, 34, 175, 154, 25, 104,
    ]),
    HexString([
        255, 215, 1, 87, 228, 128, 99, 252, 51, 201, 122, 5, 15, 127, 100, 2, 51, 191, 100, 108,
        201, 141, 149, 36, 198, 185, 43, 207, 58, 181, 111, 131,
    ]),
    HexString([
        152, 103, 204, 95, 127, 25, 107, 147, 186, 225, 226, 126, 99, 32, 116, 36, 69, 210, 144,
        242, 38, 56, 39, 73, 139, 84, 254, 197, 57, 247, 86, 175,
    ]),
    HexString([
        206, 250, 212, 229, 8, 192, 152, 185, 167, 225, 216, 254, 177, 153, 85, 251, 2, 186, 150,
        117, 88, 80, 120, 113, 9, 105, 211, 68, 15, 80, 84, 224,
    ]),
    HexString([
        249, 220, 62, 127, 224, 22, 224, 80, 239, 242, 96, 51, 79, 24, 165, 212, 254, 57, 29, 130,
        9, 35, 25, 245, 150, 79, 46, 46, 183, 193, 195, 165,
    ]),
    HexString([
        248, 177, 58, 73, 226, 130, 246, 9, 195, 23, 168, 51, 251, 141, 151, 109, 17, 81, 124, 87,
        29, 18, 33, 162, 101, 210, 90, 247, 120, 236, 248, 146,
    ]),
    HexString([
        52, 144, 198, 206, 235, 69, 10, 236, 220, 130, 226, 130, 147, 3, 29, 16, 199, 215, 59, 248,
        94, 87, 191, 4, 26, 151, 54, 10, 162, 197, 217, 156,
    ]),
    HexString([
        193, 223, 130, 217, 196, 184, 116, 19, 234, 226, 239, 4, 143, 148, 180, 211, 85, 76, 234,
        115, 217, 43, 15, 122, 249, 110, 2, 113, 198, 145, 226, 187,
    ]),
    HexString([
        92, 103, 173, 215, 198, 202, 243, 2, 37, 106, 222, 223, 122, 177, 20, 218, 10, 207, 232,
        112, 212, 73, 163, 164, 137, 247, 129, 214, 89, 232, 190, 204,
    ]),
    HexString([
        218, 123, 206, 159, 78, 134, 24, 182, 189, 47, 65, 50, 206, 121, 140, 220, 122, 96, 231,
        225, 70, 10, 114, 153, 227, 198, 52, 42, 87, 150, 38, 210,
    ]),
    HexString([
        39, 51, 229, 15, 82, 110, 194, 250, 25, 162, 43, 49, 232, 237, 80, 242, 60, 209, 253, 249,
        76, 145, 84, 237, 58, 118, 9, 162, 241, 255, 152, 31,
    ]),
    HexString([
        225, 211, 181, 200, 7, 178, 129, 228, 104, 60, 198, 214, 49, 92, 249, 91, 154, 222, 134,
        65, 222, 252, 179, 35, 114, 241, 193, 38, 227, 152, 239, 122,
    ]),
    HexString([
        90, 45, 206, 10, 138, 127, 104, 187, 116, 86, 15, 143, 113, 131, 124, 44, 46, 187, 203,
        247, 255, 251, 66, 174, 24, 150, 241, 63, 124, 116, 121, 160,
    ]),
    HexString([
        180, 106, 40, 182, 245, 85, 64, 248, 148, 68, 246, 61, 224, 55, 142, 61, 18, 27, 224, 158,
        6, 204, 157, 237, 28, 32, 230, 88, 118, 211, 106, 160,
    ]),
    HexString([
        198, 94, 150, 69, 100, 71, 134, 182, 32, 226, 221, 42, 214, 72, 221, 252, 191, 74, 126, 91,
        26, 58, 78, 207, 231, 246, 70, 103, 163, 240, 183, 226,
    ]),
    HexString([
        244, 65, 133, 136, 237, 53, 162, 69, 140, 255, 235, 57, 185, 61, 38, 241, 141, 42, 177, 59,
        220, 230, 174, 229, 142, 123, 153, 53, 158, 194, 223, 217,
    ]),
    HexString([
        90, 156, 22, 220, 0, 214, 239, 24, 183, 147, 58, 111, 141, 198, 92, 203, 85, 102, 113, 56,
        119, 111, 125, 234, 16, 16, 112, 220, 135, 150, 227, 119,
    ]),
    HexString([
        77, 248, 79, 64, 174, 12, 130, 41, 208, 214, 6, 158, 92, 143, 57, 167, 194, 153, 103, 122,
        9, 211, 103, 252, 123, 5, 227, 188, 56, 14, 230, 82,
    ]),
    HexString([
        205, 199, 37, 149, 247, 76, 123, 16, 67, 208, 225, 255, 186, 183, 52, 100, 140, 131, 141,
        251, 5, 39, 217, 113, 182, 2, 188, 33, 108, 150, 25, 239,
    ]),
    HexString([
        10, 191, 90, 201, 116, 161, 237, 87, 244, 5, 10, 165, 16, 221, 156, 116, 245, 8, 39, 123,
        57, 215, 151, 59, 178, 223, 204, 197, 238, 176, 97, 141,
    ]),
    HexString([
        184, 205, 116, 4, 111, 243, 55, 240, 167, 191, 44, 142, 3, 225, 15, 100, 44, 24, 134, 121,
        141, 113, 128, 106, 177, 232, 136, 217, 229, 238, 135, 208,
    ]),
    HexString([
        131, 140, 86, 85, 203, 33, 198, 203, 131, 49, 59, 90, 99, 17, 117, 223, 244, 150, 55, 114,
        204, 233, 16, 129, 136, 179, 74, 200, 124, 129, 196, 30,
    ]),
    HexString([
        102, 46, 228, 221, 45, 215, 178, 188, 112, 121, 97, 177, 230, 70, 196, 4, 118, 105, 220,
        182, 88, 79, 13, 141, 119, 13, 175, 93, 126, 125, 235, 46,
    ]),
    HexString([
        56, 138, 178, 14, 37, 115, 209, 113, 168, 129, 8, 231, 157, 130, 14, 152, 242, 108, 11,
        132, 170, 139, 47, 74, 164, 150, 141, 187, 129, 142, 163, 34,
    ]),
    HexString([
        147, 35, 124, 80, 186, 117, 238, 72, 95, 76, 34, 173, 242, 247, 65, 64, 11, 223, 141, 106,
        156, 199, 223, 126, 202, 229, 118, 34, 22, 101, 215, 53,
    ]),
    HexString([
        132, 72, 129, 139, 180, 174, 69, 98, 132, 158, 148, 158, 23, 172, 22, 224, 190, 22, 104,
        142, 21, 107, 92, 241, 94, 9, 140, 98, 124, 0, 86, 169,
    ]),
];

#[derive(
    borsh::BorshDeserialize, borsh::BorshSerialize, Serialize, Deserialize, Debug, PartialEq, Clone,
)]
// Incremental Merkle Tree implementation based very heavily on the implementation from cosmwasm-hyperlane:
// https://github.com/hyperlane-xyz/cosmwasm/blob/main/packages/interface/src/types/merkle.rs
pub struct MerkleTree {
    pub branch: Box<[HexHash; TREE_DEPTH]>,
    pub count: u32,
}

impl Default for MerkleTree {
    fn default() -> Self {
        Self {
            branch: Box::new(ZERO_HASHES),
            count: Default::default(),
        }
    }
}

impl MerkleTree {
    /// Insert a new leaf into the tree.
    // Compare with https://github.com/hyperlane-xyz/cosmwasm/blob/c4485e00c89c2d57315503955946bf7f155e7a47/packages/interface/src/types/merkle.rs#L68
    pub fn insert<S: Spec>(
        &mut self,
        node: HexHash,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<()> {
        self.count = self
            .count
            .checked_add(1)
            .ok_or(anyhow::anyhow!("tree is full"))?;

        let mut current_node = node;
        let mut size = self.count;
        for branch_node in self.branch.iter_mut() {
            if size & 1 == 1 {
                *branch_node = current_node;
                return Ok(());
            }
            current_node = keccak256_concat(branch_node, &current_node, gas_meter)?;
            size >>= 1;
        }
        panic!("loop above must lead to return")
    }

    /// Get the root of the tree.
    pub fn root_with_ctx<S: Spec>(
        &self,
        zeroes: &[HexHash; TREE_DEPTH],
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<HexHash> {
        let idx = self.count;
        let mut current = MerkleTree::zero();

        for (i, zero) in zeroes.iter().enumerate() {
            let ith_bit = (idx >> i) & 1;
            let next = self.branch[i];
            if ith_bit == 1 {
                current = keccak256_concat(&next, &current, gas_meter)?;
            } else {
                current = keccak256_concat(&current, zero, gas_meter)?;
            }
        }

        Ok(current)
    }

    /// Get the root of the tree.
    pub fn root<S: Spec>(&self, gas_meter: &mut impl GasMeter<Spec = S>) -> Result<HexHash> {
        self.root_with_ctx(MerkleTree::zeroes(), gas_meter)
    }

    /// Get the root of the tree at a specific index.
    pub fn branch_root<S: Spec>(
        mut item: HexHash,
        branch: &[HexHash; TREE_DEPTH],
        idx: u128,
        gas_meter: &mut impl GasMeter<Spec = S>,
    ) -> Result<HexHash> {
        for (i, next) in branch.iter().enumerate() {
            item = match (idx >> i) & 1 {
                1 => keccak256_concat(next, &item, gas_meter)?,
                _ => keccak256_concat(&item, next, gas_meter)?,
            }
        }
        Ok(item)
    }

    /// Get the zero hash.
    pub fn zero() -> HexHash {
        ZERO_BYTES
    }

    /// Get the zero hashes.
    pub fn zeroes() -> &'static [HexHash; TREE_DEPTH] {
        &ZERO_HASHES
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sov_bank::Amount;
    use sov_modules_api::{BasicGasMeter, Gas, GasArray, HexString};
    use sov_test_utils::TestSpec;

    use super::*;
    use crate::crypto::keccak256_hash;

    #[test]
    fn test_default_merkle_tree() {
        for (i, branch) in MerkleTree::default().branch.into_iter().enumerate() {
            assert_eq!(branch, super::ZERO_HASHES[i]);
        }
    }

    #[test]
    fn test_compatibility() {
        let gas_meter = &mut unlimited_gas_meter();
        let digest: HexString = [
            keccak256_hash("hello_world".as_bytes(), gas_meter)
                .unwrap()
                .0,
            keccak256_hash("world_hello".as_bytes(), gas_meter)
                .unwrap()
                .0,
        ]
        .concat()
        .into();

        assert_eq!(
            digest.to_string(),
            // abi.encodePacked(bytes32(keccak256("hello_world")), bytes32(keccak256("world_hello")));
            "0x5b07e077a81ffc6b47435f65a8727bcc542bc6fc0f25a56210efb1a74b88a5ae5e3b3917b0a11fc9edfc594b3aabbc95167d176fcc17aa76c01d7bda956862cd",
        );
    }

    #[test]
    fn test_insert() {
        let gas_meter = &mut unlimited_gas_meter();
        let mut tree = MerkleTree::default();
        for i in 0..1000 {
            tree.insert(
                keccak256_hash(i.to_string().as_bytes(), gas_meter).unwrap(),
                gas_meter,
            )
            .unwrap();
        }
        assert_eq!(tree.count, 1000);
    }

    // See <https://github.com/eigerco/hyperlane-monorepo/blob/cb6727f013e82884e15966edd863e3a888fa9184/solidity/contracts/libs/Merkle.sol#L173>
    pub const ZERO_HASHES_AS_HEX: [&str; TREE_DEPTH] = [
        "0x0000000000000000000000000000000000000000000000000000000000000000",
        "0xad3228b676f7d3cd4284a5443f17f1962b36e491b30a40b2405849e597ba5fb5",
        "0xb4c11951957c6f8f642c4af61cd6b24640fec6dc7fc607ee8206a99e92410d30",
        "0x21ddb9a356815c3fac1026b6dec5df3124afbadb485c9ba5a3e3398a04b7ba85",
        "0xe58769b32a1beaf1ea27375a44095a0d1fb664ce2dd358e7fcbfb78c26a19344",
        "0x0eb01ebfc9ed27500cd4dfc979272d1f0913cc9f66540d7e8005811109e1cf2d",
        "0x887c22bd8750d34016ac3c66b5ff102dacdd73f6b014e710b51e8022af9a1968",
        "0xffd70157e48063fc33c97a050f7f640233bf646cc98d9524c6b92bcf3ab56f83",
        "0x9867cc5f7f196b93bae1e27e6320742445d290f2263827498b54fec539f756af",
        "0xcefad4e508c098b9a7e1d8feb19955fb02ba9675585078710969d3440f5054e0",
        "0xf9dc3e7fe016e050eff260334f18a5d4fe391d82092319f5964f2e2eb7c1c3a5",
        "0xf8b13a49e282f609c317a833fb8d976d11517c571d1221a265d25af778ecf892",
        "0x3490c6ceeb450aecdc82e28293031d10c7d73bf85e57bf041a97360aa2c5d99c",
        "0xc1df82d9c4b87413eae2ef048f94b4d3554cea73d92b0f7af96e0271c691e2bb",
        "0x5c67add7c6caf302256adedf7ab114da0acfe870d449a3a489f781d659e8becc",
        "0xda7bce9f4e8618b6bd2f4132ce798cdc7a60e7e1460a7299e3c6342a579626d2",
        "0x2733e50f526ec2fa19a22b31e8ed50f23cd1fdf94c9154ed3a7609a2f1ff981f",
        "0xe1d3b5c807b281e4683cc6d6315cf95b9ade8641defcb32372f1c126e398ef7a",
        "0x5a2dce0a8a7f68bb74560f8f71837c2c2ebbcbf7fffb42ae1896f13f7c7479a0",
        "0xb46a28b6f55540f89444f63de0378e3d121be09e06cc9ded1c20e65876d36aa0",
        "0xc65e9645644786b620e2dd2ad648ddfcbf4a7e5b1a3a4ecfe7f64667a3f0b7e2",
        "0xf4418588ed35a2458cffeb39b93d26f18d2ab13bdce6aee58e7b99359ec2dfd9",
        "0x5a9c16dc00d6ef18b7933a6f8dc65ccb55667138776f7dea101070dc8796e377",
        "0x4df84f40ae0c8229d0d6069e5c8f39a7c299677a09d367fc7b05e3bc380ee652",
        "0xcdc72595f74c7b1043d0e1ffbab734648c838dfb0527d971b602bc216c9619ef",
        "0x0abf5ac974a1ed57f4050aa510dd9c74f508277b39d7973bb2dfccc5eeb0618d",
        "0xb8cd74046ff337f0a7bf2c8e03e10f642c1886798d71806ab1e888d9e5ee87d0",
        "0x838c5655cb21c6cb83313b5a631175dff4963772cce9108188b34ac87c81c41e",
        "0x662ee4dd2dd7b2bc707961b1e646c4047669dcb6584f0d8d770daf5d7e7deb2e",
        "0x388ab20e2573d171a88108e79d820e98f26c0b84aa8b2f4aa4968dbb818ea322",
        "0x93237c50ba75ee485f4c22adf2f741400bdf8d6a9cc7df7ecae576221665d735",
        "0x8448818bb4ae4562849e949e17ac16e0be16688e156b5cf15e098c627c0056a9",
    ];

    #[test]
    fn test_zeros() {
        let zeros = MerkleTree::zeroes();
        for (i, actual) in zeros.iter().enumerate() {
            let expected = HexString::from_str(ZERO_HASHES_AS_HEX[i]).unwrap();
            assert_eq!(
                actual, &expected,
                "Bad hash at index {i}. Expected {expected} but got {actual}"
            );
        }
    }

    fn unlimited_gas_meter() -> BasicGasMeter<TestSpec> {
        BasicGasMeter::new_with_funds_and_gas(
            Amount::MAX,
            <<TestSpec as Spec>::Gas as Gas>::max(),
            <<TestSpec as Spec>::Gas as Gas>::Price::ZEROED,
        )
    }
}
