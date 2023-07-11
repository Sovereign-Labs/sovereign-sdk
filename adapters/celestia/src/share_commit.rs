use tendermint::crypto::default::Sha256;
use tendermint::merkle::simple_hash_from_byte_vectors;

use crate::shares::{self, Share};

// /// Calculates the size of the smallest square that could be used to commit
// /// to this message, following Celestia's "non-interactive default rules"
// /// https://github.com/celestiaorg/celestia-app/blob/fbfbf111bcaa056e53b0bc54d327587dee11a945/docs/architecture/adr-008-blocksize-independent-commitment.md
// fn min_square_size(message: &[u8]) -> usize {
//     let square_size = message.len().next_power_of_two();
//     if message.len() < (square_size * square_size - 1) {
//         return square_size;
//     } else {
//         return square_size << 1;
//     }
// }

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CommitmentError {
    ErrMessageTooLarge,
}

impl std::fmt::Display for CommitmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ErrMessageTooLarge")
    }
}

impl std::error::Error for CommitmentError {}

/// Derived from https://github.com/celestiaorg/celestia-app/blob/0c81704939cd743937aac2859f3cb5ae6368f174/x/payment/types/payfordata.go#L170
pub fn recreate_commitment(
    square_size: usize,
    shares: shares::BlobRef,
) -> Result<[u8; 32], CommitmentError> {
    if shares.0.len() > (square_size * square_size) - 1 {
        return Err(CommitmentError::ErrMessageTooLarge);
    }

    let heights = power_of_2_mountain_range(shares.0.len(), square_size);
    let mut leaf_sets: Vec<&[Share]> = Vec::with_capacity(heights.len());
    let mut cursor = 0;
    for height in heights {
        leaf_sets.push(&shares.0[cursor..cursor + height]);
        cursor += height;
    }

    let mut subtree_roots = Vec::with_capacity(leaf_sets.len());
    for set in leaf_sets {
        let mut tree = nmt_rs::CelestiaNmt::new();
        for share in set {
            let nid = share.namespace();
            tree.push_leaf(share.as_serialized(), nid)
                .expect("Leaves are pushed in order");
        }
        subtree_roots.push(tree.root());
    }
    let h = simple_hash_from_byte_vectors::<Sha256>(&subtree_roots);
    Ok(h)
}

// power_of_2_mountain_range returns the heights of the subtrees for binary merkle
// mountain range
fn power_of_2_mountain_range(mut len: usize, square_size: usize) -> Vec<usize> {
    let mut output = Vec::new();

    while len != 0 {
        if len >= square_size {
            output.push(square_size);
            len -= square_size;
        } else {
            let p = next_lower_power_of_2(len);
            output.push(p);
            len -= p;
        }
    }
    output
}

/// returns the largest power of 2 that is less than or equal to the input
/// Examples:
///   - next_lower_power_of_2(2): 2
///   - next_lower_power_of_2(3): 2
///   - next_lower_power_of_2(7): 4
///   - next_lower_power_of_2(8): 8
fn next_lower_power_of_2(num: usize) -> usize {
    if num.is_power_of_two() {
        num
    } else {
        num.next_power_of_two() >> 1
    }
}

mod nmt {
    // /// Build an nmt from leaves that are already prefixed with their namespace
    // pub fn build_nmt_from_namespaced_leaves(namespaced_leaves: &[impl AsRef<[u8]>]) -> [u8; 48] {
    //     let mut tree = CelestiaNmt::new();
    //     for leaf in namespaced_leaves.iter() {
    //         let namespace: NamespaceId = leaf.as_ref()[..8]
    //             .as_ref()
    //             .try_into()
    //             .expect("Namespace length is correct");
    //         tree.push_leaf(&leaf.as_ref()[8..], namespace)
    //             .expect("Leaves are pushed in order");
    //     }
    //     tree.root().0
    // }

    // pub fn build_nmt(leaves: &[(impl AsRef<[u8]>, NamespaceId)]) -> [u8; 48] {
    //     let mut tree = CelestiaNmt::new();
    //     for (leaf, ns) in leaves {
    //         tree.push_leaf(leaf.as_ref(), *ns);
    //     }

    //     tree.root().0
    // }
}
