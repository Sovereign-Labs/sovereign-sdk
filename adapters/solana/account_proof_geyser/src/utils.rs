use std::collections::HashMap;

use blake3::traits::digest::Digest;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use rayon::prelude::*;
use solana_runtime::accounts_hash::{AccountsHasher, MERKLE_FANOUT};
use solana_sdk::hash::{Hash, Hasher, hashv};
use solana_sdk::pubkey::Pubkey;

use crate::types::{AccountDeltaProof, AccountHashMap, Data, Proof};

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

// Simple wrapper around the solana function
pub fn calculate_root(pubkey_hash_vec: Vec<(Pubkey, Hash)>) -> Hash {
    AccountsHasher::accumulate_account_hashes(pubkey_hash_vec)
}

// Originally attempted to calculate the proof while generating the root
// but logically felt more complex, so the root calculation is separated from proof gen
// TODO: see if these can be combined
pub fn calculate_root_and_proofs(
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

pub fn are_adjacent(proof1: &Proof, proof2: &Proof) -> bool {
    if proof1.path.len() != proof2.path.len() {
        println!(
            "Proofs have different path lengths: {} vs {}",
            proof1.path.len(),
            proof2.path.len()
        );
        return false;
    }

    for i in 0..proof1.path.len() {
        if proof1.path[i] != proof2.path[i] {
            let divergence = (proof1.path[i] as i32 - proof2.path[i] as i32).abs();

            if divergence != 1 && divergence != ((MERKLE_FANOUT - 1) as i32) {
                println!(
                    "Proofs diverge at position {}: proof1[{}]={}, proof2[{}]={}",
                    i, i, proof1.path[i], i, proof2.path[i]
                );
                return false;
            }
        }
    }
    true
}

pub fn is_first(proof: &Proof) -> bool {
    proof.path.iter().all(|&position| position == 0)
}

pub fn get_proof_pubkeys_required(
    pubkey_hash_vec: &mut [(Pubkey, Hash)],
    leaves_for_proof: &[Pubkey],
) -> (Vec<Pubkey>, Vec<Pubkey>, Vec<Pubkey>, Vec<Pubkey>) {
    pubkey_hash_vec.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let smallest_key_in_hash_vec = pubkey_hash_vec.first().map(|(pubkey, _)| pubkey).unwrap();
    let largest_key_in_hash_vec = pubkey_hash_vec.last().map(|(pubkey, _)| pubkey).unwrap();

    let mut inclusion = vec![];
    let mut non_inclusion_left = vec![];
    let mut non_inclusion_right = vec![];
    let mut non_inclusion_inner = vec![];

    for &leaf in leaves_for_proof {
        if pubkey_hash_vec.iter().any(|(pubkey, _)| &leaf == pubkey) {
            inclusion.push(leaf);
        } else if leaf < *smallest_key_in_hash_vec {
            non_inclusion_left.push(leaf);
        } else if leaf > *largest_key_in_hash_vec {
            non_inclusion_right.push(leaf);
        } else {
            non_inclusion_inner.push(leaf);
        }
    }

    (
        inclusion,
        non_inclusion_left,
        non_inclusion_right,
        non_inclusion_inner,
    )
}

pub fn get_keys_for_non_inclusion_inner(
    non_inclusion_inner: &[Pubkey],
    pubkey_hash_vec: &mut [(Pubkey, Hash)],
) -> (Vec<Pubkey>, HashMap<Pubkey, (Pubkey, Pubkey)>) {
    pubkey_hash_vec.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let sorted_keys: Vec<&Pubkey> = pubkey_hash_vec.iter().map(|(pubkey, _)| pubkey).collect();

    let mut adjacent_pairs = Vec::new();
    let mut missing_key_to_adjacent = HashMap::new();

    for &missing_key in non_inclusion_inner {
        let position = match sorted_keys.binary_search(&&missing_key) {
            Ok(pos) => pos, // This shouldn't really happen, but in case it does
            Err(pos) => pos,
        };

        if position > 0 && position < sorted_keys.len() {
            let previous_key = sorted_keys[position - 1];
            let next_key = sorted_keys[position];

            // Add adjacent keys to the result vector
            if !adjacent_pairs.contains(previous_key) {
                adjacent_pairs.push(*previous_key);
            }
            if !adjacent_pairs.contains(next_key) {
                adjacent_pairs.push(*next_key);
            }

            // Associate the missing key with its adjacent keys in the HashMap
            missing_key_to_adjacent.insert(missing_key, (*previous_key, *next_key));
        }
    }

    (adjacent_pairs, missing_key_to_adjacent)
}

pub fn assemble_account_delta_proof(
    account_hashes: &[(Pubkey, Hash)],
    account_data_hashes: &AccountHashMap,
    account_proofs: &[(Pubkey, Proof)],
    inclusion: &[Pubkey],
    non_inclusion_left: &[Pubkey],
    non_inclusion_right: &[Pubkey],
    non_inclusion_inner: &[Pubkey],
    non_inclusion_inner_mapping: &HashMap<Pubkey, (Pubkey, Pubkey)>,
) -> anyhow::Result<Vec<AccountDeltaProof>> {
    let account_proofs_map: HashMap<Pubkey, Proof> = account_proofs.iter().cloned().collect();

    let mut proofs = vec![];
    for incl in inclusion {
        let data = Data {
            pubkey: incl.clone(),
            hash: account_data_hashes.get(&incl).unwrap().1,
            account: account_data_hashes.get(&incl).unwrap().2.clone(),
        };
        let account_proof = AccountDeltaProof::InclusionProof(
            incl.clone(),
            (data, account_proofs_map.get(&incl).unwrap().clone()),
        );
        proofs.push(account_proof)
    }

    for nil in non_inclusion_left {
        let data = Data {
            pubkey: nil.clone(),
            hash: account_data_hashes.get(&nil).unwrap().1,
            account: account_data_hashes.get(&nil).unwrap().2.clone(),
        };
        let account_proof = AccountDeltaProof::NonInclusionProofLeft(
            nil.clone(),
            (
                data,
                account_proofs_map
                    .get(&account_hashes[0].0)
                    .unwrap()
                    .clone(),
            ),
        );
        proofs.push(account_proof)
    }

    for nii in non_inclusion_inner {
        let (nii_l, nii_r) = non_inclusion_inner_mapping.get(nii).unwrap();
        let data_l = Data {
            pubkey: nii_l.clone(),
            hash: account_data_hashes.get(&nii_l).unwrap().1,
            account: account_data_hashes.get(&nii_l).unwrap().2.clone(),
        };
        let data_r = Data {
            pubkey: nii_r.clone(),
            hash: account_data_hashes.get(&nii_r).unwrap().1,
            account: account_data_hashes.get(&nii_r).unwrap().2.clone(),
        };
        let account_proof = AccountDeltaProof::NonInclusionProofInner(
            nii.clone(),
            (
                (data_l, account_proofs_map.get(nii_l).unwrap().clone()),
                (data_r, account_proofs_map.get(nii_r).unwrap().clone()),
            ),
        );
        proofs.push(account_proof)
    }

    for nir in non_inclusion_right {
        let last_pubkey = account_hashes[account_hashes.len() - 1].0;
        let all_hashes: Vec<Hash> = account_hashes.iter().map(|x| x.1).collect();
        let data = Data {
            pubkey: last_pubkey.clone(),
            hash: account_data_hashes.get(&last_pubkey).unwrap().1,
            account: account_data_hashes.get(&last_pubkey).unwrap().2.clone(),
        };
        let account_proof = AccountDeltaProof::NonInclusionProofRight(
            nir.clone(),
            (
                data,
                account_proofs_map.get(&last_pubkey).unwrap().clone(),
                all_hashes.clone(),
            ),
        );
        proofs.push(account_proof)
    }

    Ok(proofs)
}

pub fn verify_leaves_against_bankhash(account_proof: AccountDeltaProof,
                                      bankhash: Hash,
                                      num_sigs: u64,
                                      account_delta_root: Hash,
                                      parent_bankhash: Hash,
                                      blockhash: Hash) -> anyhow::Result<()> {

    match account_proof {
        AccountDeltaProof::InclusionProof(pubkey, (data, proof)) => {
            if data.account.pubkey != pubkey {
                anyhow::bail!("account info pubkey doesn't match pubkey in provided update");
            }
            if data.hash.as_ref() != hash_solana_account(
                data.account.lamports,
                data.account.owner.as_ref(),
                data.account.executable,
                data.account.rent_epoch,
                &data.account.data,
                data.account.pubkey.as_ref()) {
                anyhow::bail!("account data does not match account hash");
            }
            if bankhash != hashv(&[
                parent_bankhash.as_ref(),
                account_delta_root.as_ref(),
                &num_sigs.to_le_bytes(),
                blockhash.as_ref(),
            ]) {
                anyhow::bail!("bank hash does not match data");
            }
            if !verify_proof(&data.hash, &proof, &account_delta_root) {
                anyhow::bail!("account merkle proof verification failure");
            }
            Ok(())
        }
        _ => {
            anyhow::bail!("Only Inclusion proof");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use rand::Rng;
    use solana_sdk::hash::hashv;

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

        pubkey_hash_vec.par_sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let random_index = rng.gen_range(2..pubkey_hash_vec.len() - 3); // "- 2" to avoid picking the last element.
        println!("{}", random_index);
        let mut proof_leaves: Vec<_> = (random_index..random_index + 3)
            .map(|i| pubkey_hash_vec[i].0.clone())
            .collect();
        let first_leaf = pubkey_hash_vec[0].0.clone();
        let last_leaf = pubkey_hash_vec[pubkey_hash_vec.len() - 1].0.clone();
        let inner_leaves = proof_leaves.clone();
        proof_leaves.push(first_leaf);
        proof_leaves.push(last_leaf);
        let (root, proofs) = calculate_root_and_proofs(&mut pubkey_hash_vec, &proof_leaves);

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
        let first_leaf_proof = proofs
            .iter()
            .find(|(k, _)| *k == first_leaf)
            .unwrap()
            .1
            .clone();
        let last_leaf_proof = proofs
            .iter()
            .find(|(k, _)| *k == last_leaf)
            .unwrap()
            .1
            .clone();

        let inner_1 = proofs
            .iter()
            .find(|(k, _)| *k == inner_leaves[0])
            .unwrap()
            .1
            .clone();
        let inner_2 = proofs
            .iter()
            .find(|(k, _)| *k == inner_leaves[1])
            .unwrap()
            .1
            .clone();
        let inner_3 = proofs
            .iter()
            .find(|(k, _)| *k == inner_leaves[2])
            .unwrap()
            .1
            .clone();

        println!("{:?}", inner_leaves);

        assert!(are_adjacent(&inner_1, &inner_2));
        assert!(are_adjacent(&inner_2, &inner_1));
        assert!(!are_adjacent(&inner_1, &inner_3));
        assert!(!are_adjacent(&inner_3, &inner_1));

        assert!(is_first(&first_leaf_proof));
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

        let (root, mut proofs) = calculate_root_and_proofs(&mut pubkey_hash_vec, &proof_leaves);

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
