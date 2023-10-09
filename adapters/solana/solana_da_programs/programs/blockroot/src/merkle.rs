use anchor_lang::solana_program;
use solana_program::keccak::hashv;

pub fn compute_merkle_root(hashes: &Vec<[u8; 32]>) -> [u8; 32] {
    let mut current_level = hashes.clone();

    while current_level.len() > 1 {
        let mut new_level = Vec::new();

        for nodes in current_level.chunks(2) {
            if nodes.len() == 2 {
                let combined = hashv(&[&nodes[0], &nodes[1]]);
                new_level.push(combined.to_bytes());
            } else {
                // If there's a single item (odd number of nodes), push it to the next level as-is.
                new_level.push(nodes[0]);
            }
        }

        current_level = new_level;
    }

    current_level[0]
}
