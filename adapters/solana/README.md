### Solana DA
This folder contains
1. On chain program that enables using Solana as a minimal DA layer (solana/solana_da_programs)
2. Client to interact with the on-chain program (solana/da_client)

## Installation
* `solana_da_programs` is an `anchor` workspace and requires `anchor` installed to make use of it.
  * https://book.anchor-lang.com/getting_started/installation.html
  * ensure that anchor is also installed with avm
* solana cli and solana-test-validator are both used for testing. 
  * https://docs.solana.com/cli/install-solana-cli-tools

## Process

### Preparation
* The on-chain program that handles the DA logic is located at `solana/solana_da_programs/programs/bockroot`
* A sequencer that wants to submit blobs for rollup first needs to create a `ChunkAccumulator` account on-chain
  * This account is like "scratch space" for the sequencer. It keeps track of chunks as they arrive
  * The account is a keypair account
    * Solana has two kinds of accounts
      * keypair accounts: these are keys where the public key is a point on the curve
      * PDA: This is an address without a private key (since the point does not lie on the curve)
    * Keypair accounts are associated with a private key, but they can also be owned by a program
    * Only a program that owns an account can modify its contents
    * The reason we use Keypair account for scratch space is because Keypair accounts have a size limit of 10MB whereas PDAs are restricted to 10KB
* As part of creating the `ChunkAccumulator` account, the instruction also includes a transfer of ownership to the `blockroot` program
* Once the account is created, the sequencer calls the `Initialize` with the account as Input.
  * This initializes the account data with the base structures needed to track chunks.
* The keypair needs to be retained since all interactions with the `blockroot` program require two signatures
  * `ChunkAccumulator` signature to prove ownership of the account. This is so that each sequencer can have its own "scratch space" and other sequencers cannot modify it using the program
  * The transaction signer which pays the gas fees

### Submitting blobs
* A rollup blob is a sequence of bytes `&[u8]`
* The blob is broken down into chunks using the `get_chunks` function
```rust
pub struct Chunk {
    pub digest: [u8; 32],
    pub num_chunks: u64,
    pub chunk_num: u64,
    pub actual_size: u64,
    pub chunk_body: Vec<u8>,
}
```
  * The actual data of the blob is part of `chunk_body`
  * The remaining fields are metadata
    * `digest`: This is a unique identifier for the blob and serves to indicate which blob a specific chunk belongs to. This can be any unique identifier, but for convenience, it's currently the merkle root of all the chunks
    * `num_chunks`: Number of chunks that constitute the blob
    * `chunk_num`: The position in the sequence of chunks that form blob with `digest`. Used to order the chunks in order to reconstruct the blob
    * `actual_size`: The chunks are all of equal size, so the final chunk has padding. `actual_size` is used to enable stripping out padding during reconstruction.
      * We can do away with padding if we find that it's un-necessary.
* The `blockroot` program contains 3 instructions
  * Initialize - used to initialize the accounts
  * Clear - used to clear the `ChunkAccumulator` account of any incomplete blobs
  * ProcessChunk - logic to keep track of the chunks as they arrive and calculate the merkle root of the chunks
* 