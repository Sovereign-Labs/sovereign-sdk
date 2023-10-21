use std::collections::HashMap;
use std::str::FromStr;

use blake3::traits::digest::Digest;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rayon::prelude::*;
use solana_runtime::accounts_hash::{AccountsHasher, MERKLE_FANOUT};
use solana_sdk::hash::{hashv, Hash, Hasher};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;

/// Util helper function to calculate the hash of a solana account
/// https://github.com/solana-labs/solana/blob/v1.16.15/runtime/src/accounts_db.rs#L6076-L6118
/// We can see as we make the code more resilient to see if we can also make
/// the structures match and use the function from solana-sdk, but currently it seems a bit more
/// complicated and lower priority, since getting a stable version working is top priority
pub fn hash_solana_account(
    lamports: u64,
    owner: &[u8],
    executable: bool,
    rent_epoch: u64,
    data: &[u8],
    pubkey: &[u8],
) -> [u8; 32] {
    if lamports == 0 {
        return [08; 32];
    }
    let mut hasher = blake3::Hasher::new();

    hasher.update(&lamports.to_le_bytes());
    hasher.update(&rent_epoch.to_le_bytes());
    hasher.update(data);

    if executable {
        hasher.update(&[1u8; 1]);
    } else {
        hasher.update(&[0u8; 1]);
    }
    hasher.update(owner.as_ref());
    hasher.update(pubkey.as_ref());

    hasher.finalize().into()
}

pub fn calculate_root(pubkey_hash_vec: Vec<(Pubkey, Hash)>) -> Hash {
    AccountsHasher::accumulate_account_hashes(pubkey_hash_vec)
}

// Solana MERKLE_FANOUT is 16, so this logic needs to handle more siblings
#[derive(Clone, Debug)]
pub struct Proof {
    pub path: Vec<usize>, // Position in the chunk (between 0 and 15) for each level.
    pub siblings: Vec<Vec<Hash>>, // Sibling hashes at each level.
}

pub fn calculate_root_custom(
    pubkey_hash_vec: &mut [(Pubkey, Hash)],
    leaves_for_proof: &[Pubkey],
) -> (Hash, Vec<(Pubkey, Proof)>) {
    pubkey_hash_vec.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let root = compute_merkle_root_loop(pubkey_hash_vec, MERKLE_FANOUT, |i: &(Pubkey, Hash)| &i.1);
    let proofs = generate_merkle_proofs(pubkey_hash_vec, leaves_for_proof);

    (root, proofs)
}

pub fn generate_merkle_proofs(
    pubkey_hash_vec: &[(Pubkey, Hash)],
    leaves_for_proof: &[Pubkey],
) -> Vec<(Pubkey, Proof)> {
    let mut proofs = Vec::new();

    for &key in leaves_for_proof {
        let mut path = Vec::new();
        let mut siblings = Vec::new();

        // Find the position of the key in the sorted pubkey_hash_vec
        let mut pos = pubkey_hash_vec
            .binary_search_by(|&(ref k, _)| k.cmp(&key))
            .unwrap();

        let mut current_hashes: Vec<_> = pubkey_hash_vec
            .iter()
            .map(|&(_, ref h)| h.clone())
            .collect();
        while current_hashes.len() > 1 {
            let chunk_index = pos / MERKLE_FANOUT;
            let index_in_chunk = pos % MERKLE_FANOUT;

            path.push(index_in_chunk);

            // Collect the hashes of the siblings for the current hash in this level.
            let mut sibling_hashes = Vec::with_capacity(MERKLE_FANOUT - 1);
            for i in 0..MERKLE_FANOUT {
                if i == index_in_chunk {
                    continue;
                }
                let sibling_pos = chunk_index * MERKLE_FANOUT + i;
                if sibling_pos < current_hashes.len() {
                    sibling_hashes.push(current_hashes[sibling_pos].clone());
                }
            }
            siblings.push(sibling_hashes);

            // Move up one level in the tree.
            current_hashes = compute_hashes_at_next_level(&current_hashes);
            pos = chunk_index;
        }

        proofs.push((key, Proof { path, siblings }));
    }

    proofs
}

fn compute_hashes_at_next_level(hashes: &[Hash]) -> Vec<Hash> {
    let chunks = div_ceil(hashes.len(), MERKLE_FANOUT);
    (0..chunks)
        .map(|i| {
            let start_index = i * MERKLE_FANOUT;
            let end_index = std::cmp::min(start_index + MERKLE_FANOUT, hashes.len());

            let mut hasher = Hasher::default();
            for hash in &hashes[start_index..end_index] {
                hasher.hash(hash.as_ref());
            }

            hasher.result()
        })
        .collect()
}

pub fn compute_merkle_root_loop<T, F>(hashes: &[T], fanout: usize, extractor: F) -> Hash
where
    F: Fn(&T) -> &Hash + std::marker::Sync,
    T: std::marker::Sync,
{
    if hashes.is_empty() {
        return Hasher::default().result();
    }

    let total_hashes = hashes.len();
    let chunks = div_ceil(total_hashes, fanout);

    let result: Vec<_> = (0..chunks)
        .into_par_iter()
        .map(|i| {
            let start_index = i * fanout;
            let end_index = std::cmp::min(start_index + fanout, total_hashes);

            let mut hasher = Hasher::default();
            for item in hashes.iter().take(end_index).skip(start_index) {
                let h = extractor(item);
                hasher.hash(h.as_ref());
            }

            hasher.result()
        })
        .collect();

    if result.len() == 1 {
        result[0]
    } else {
        compute_merkle_root_recurse(&result, fanout)
    }
}

// this function avoids an infinite recursion compiler error
pub fn compute_merkle_root_recurse(hashes: &[Hash], fanout: usize) -> Hash {
    compute_merkle_root_loop(hashes, fanout, |t| t)
}

pub fn div_ceil(x: usize, y: usize) -> usize {
    let mut result = x / y;
    if x % y != 0 {
        result += 1;
    }
    result
}

pub fn verify_proof(leaf_hash: &Hash, proof: &Proof, root: &Hash) -> bool {
    // Validate path length and siblings length
    if proof.path.len() != proof.siblings.len() {
        return false;
    }

    let mut current_hash = leaf_hash.clone();

    for (index_in_chunk, sibling_hashes) in proof.path.iter().zip(&proof.siblings) {
        let mut hasher = Hasher::default();

        // We need to hash the elements in the correct order.
        // Before the current hash, add the siblings.
        for i in 0..*index_in_chunk {
            hasher.hash(sibling_hashes[i].as_ref());
        }

        // Hash the current hash
        hasher.hash(current_hash.as_ref());

        // After the current hash, add the remaining siblings.
        for i in *index_in_chunk..sibling_hashes.len() {
            hasher.hash(sibling_hashes[i].as_ref());
        }

        current_hash = hasher.result();
    }

    &current_hash == root
}

fn are_adjacent(proof1: &Proof, proof2: &Proof) -> bool {
    if proof1.path.len() != proof2.path.len() {
        return false;
    }

    // Check if proof1 represents the first leaf
    if proof1.path.iter().all(|&position| position == 0) {
        // If proof2 is the next leaf after the first one
        return proof2.path[..proof2.path.len() - 1]
            .iter()
            .all(|&position| position == 0)
            && proof2.path.last().unwrap() == &1;
    }

    // Check if proof2 represents the first leaf
    if proof2.path.iter().all(|&position| position == 0) {
        // If proof1 is the next leaf after the first one
        return proof1.path[..proof1.path.len() - 1]
            .iter()
            .all(|&position| position == 0)
            && proof1.path.last().unwrap() == &1;
    }

    // Check if proof1 represents the last leaf
    if proof1
        .path
        .iter()
        .all(|&position| position == MERKLE_FANOUT - 1)
    {
        // If proof2 is the leaf just before the last one
        return proof2.path[..proof2.path.len() - 1]
            .iter()
            .all(|&position| position == MERKLE_FANOUT - 1)
            && proof2.path.last().unwrap() == &(MERKLE_FANOUT - 2);
    }

    // Check if proof2 represents the last leaf
    if proof2
        .path
        .iter()
        .all(|&position| position == MERKLE_FANOUT - 1)
    {
        // If proof1 is the leaf just before the last one
        return proof1.path[..proof1.path.len() - 1]
            .iter()
            .all(|&position| position == MERKLE_FANOUT - 1)
            && proof1.path.last().unwrap() == &(MERKLE_FANOUT - 2);
    }

    // Check for regular adjacency (neither are first or last leaves)
    for i in 0..proof1.path.len() {
        if proof1.path[i] != proof2.path[i] {
            // If they diverge by more than one position, they are not adjacent
            if i == proof1.path.len() - 1
                || (proof1.path[i] as i32 - proof2.path[i] as i32).abs() != 1
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use rand::Rng;

    use super::*;

    fn generate_random_pubkey() -> Pubkey {
        let random_bytes: [u8; 32] = rand::thread_rng().gen();
        Pubkey::try_from(random_bytes).unwrap()
    }

    fn generate_random_hash() -> Hash {
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        hashv(&[&random_bytes])
    }

    #[test]
    fn test_proof_verification() {
        let mut pubkey_hash_vec: Vec<(Pubkey, Hash)> = (0..1000)
            .map(|_| (generate_random_pubkey(), generate_random_hash()))
            .collect();

        let mut rng = rand::thread_rng();
        let random_indices: Vec<_> = (0..3)
            .map(|_| rng.gen_range(0..pubkey_hash_vec.len()))
            .collect();
        let proof_leaves: Vec<_> = random_indices
            .iter()
            .map(|&i| pubkey_hash_vec[i].0.clone())
            .collect();

        let (root, proofs) = calculate_root_custom(&mut pubkey_hash_vec, &proof_leaves);

        for (pubkey, proof) in &proofs {
            let leaf_hash = pubkey_hash_vec
                .iter()
                .find(|(k, _)| k == pubkey)
                .unwrap()
                .1
                .clone();
            assert!(verify_proof(&leaf_hash, proof, &root));
        }

        let solana_root = calculate_root(pubkey_hash_vec);

        assert_eq!(solana_root, root);
    }

    #[test]
    fn test_invalid_proof_verification() {
        let mut pubkey_hash_vec: Vec<(Pubkey, Hash)> = (0..1000)
            .map(|_| (generate_random_pubkey(), generate_random_hash()))
            .collect();

        let mut rng = rand::thread_rng();
        let random_indices: Vec<_> = (0..3)
            .map(|_| rng.gen_range(0..pubkey_hash_vec.len()))
            .collect();
        let proof_leaves: Vec<_> = random_indices
            .iter()
            .map(|&i| pubkey_hash_vec[i].0.clone())
            .collect();

        let (root, mut proofs) = calculate_root_custom(&mut pubkey_hash_vec, &proof_leaves);

        // Keep a copy of the original proof for comparison
        let original_proof = proofs[0].1.clone();

        // Modify one of the proofs to make it invalid
        if let Some((_pubkey, proof)) = proofs.iter_mut().next() {
            if !proof.path.is_empty() {
                proof.path[0] = (proof.path[0] + 1) % MERKLE_FANOUT; // Change the path slightly to invalidate it
            }
        }

        // Print the modified and original proofs for comparison
        println!("Original Proof: {:?}", original_proof);
        println!("Modified Proof: {:?}", proofs[0].1);

        // Verify the proofs
        for (idx, (pubkey, proof)) in proofs.iter().enumerate() {
            let leaf_hash = pubkey_hash_vec
                .iter()
                .find(|(k, _)| k == pubkey)
                .unwrap()
                .1
                .clone();

            // Diagnostic
            println!(
                "\nVerifying proof for pubkey at index {}: {:?}",
                random_indices[idx], pubkey
            );

            let verification_result = verify_proof(&leaf_hash, proof, &root);

            // Diagnostic
            println!("Leaf Hash: {:?}", leaf_hash);
            println!("Root: {:?}", root);
            println!("Verification Result: {}", verification_result);

            // Check that we're testing the modified proof and assert accordingly
            if proof.path == proofs[0].1.path {
                println!("Testing modified proof...");
                assert!(!verification_result);
            } else {
                println!("Testing non-modified proof...");
                assert!(verification_result);
            }
        }
    }
}
