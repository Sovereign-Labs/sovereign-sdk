use anchor_lang::solana_program;
use solana_program::keccak::hashv;

pub fn compute_merkle_root(data: Vec<&[u8]>) -> [u8;32] {
    let mut hashes = data.iter().map(|item| hashv(&[*item])).collect::<Vec<_>>();

    while hashes.len() > 1 {
        if hashes.len() % 2 != 0 {
            hashes.push(hashes.last().unwrap().clone());
        }

        let mut new_hashes = Vec::new();
        for i in (0..hashes.len()).step_by(2) {
            let combined = hashv(&[&hashes[i].as_ref(), &hashes[i+1].as_ref()]);
            new_hashes.push(combined);
        }

        hashes = new_hashes;
    }

    hashes[0].to_bytes()
}
