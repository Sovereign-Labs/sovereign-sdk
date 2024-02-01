use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use blake3::traits::digest::Digest;
use solana_runtime::accounts_hash::AccountsHasher;
use solana_sdk::hash::Hash;
use solana_sdk::pubkey::Pubkey;

/// Util helper function to write `size` number of random bytes to file at path `P`
pub fn write_random_bytes<P: AsRef<Path>>(path: P, size: u64) -> std::io::Result<()> {
    let mut file = File::create(path)?;

    let random_bytes: Vec<u8> = (0..size).map(|_| rand::random::<u8>()).collect();

    file.write_all(&random_bytes)
}

/// Util helper function to read file at path `P` as bytes
pub fn read_file_to_vec<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();

    file.read_to_end(&mut buffer)?;

    Ok(buffer)
}

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
