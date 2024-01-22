## Background
In order to prove data availability, we have an on-chain program that accepts chunks, calculates the merkle root once all the chunks are received on-chain and then updates a Program Derived Address (PDA) with the merkle root.
Solana does not have full state commitments every block, instead it has a commitment to the accounts that were modified within that block as part of the BankHash.
Since the PDA is modified everytime a full rollup blob is seen by solana (i.e. all the chunks), the root of the chunks will be committed to in the BankHash.
Details about how the PDA is modified are available in [README](README.md)

### BankHash
* The BankHash is the commitment chosen by the staked validators to vote on
* The BankHash is created by hashing the following components
```
let mut hash = hashv(&[
    self.parent_hash.as_ref(),
    accounts_delta_hash.0.as_ref(),
    &signature_count_buf,
    self.last_blockhash().as_ref(),
]);
```
 * `parent_hash` refers to the parent bankhash
 * `accounts_delta_hash` refers to the merkle root of the modified accounts in that block (these are sorted by account address)
 * `signature_count_buf` is the number of signatures in the block
 * `last_blockhash` is the "blockhash" - it's different from the bankhash and refers to the last PoH tick after interleaving all the transactions together.

### Clarification about terminology
* Solana uses multiple terms `slothash`, `bankhash`, `blockhash`
* `slothash` is the same as `bankhash`. 
* `blockhash` refers to a PoH tick
* However, the json rpc is mis-leading. `getBlock` for instance returns data of this example format
```json
{
  "jsonrpc": "2.0",
  "result": {
    "blockHeight": 428,
    "blockTime": null,
    "blockhash": "3Eq21vXNB5s86c62bVuUfTeaMif1N2kUqRPBmGRJhyTA",
    "parentSlot": 429,
    "previousBlockhash": "mfcyqEXB3DnHXki6KjjmZck6YjmZLvpAByy2fj4nh6B",
    "transactions": [
    ]
  },
  "id": 1
}
```
* In the above response `previousBlockhash` actually refers to the BankHash after the previous block was applied. It corresponds to `parent_hash` used in the previous section
* `blockhash` however corresponds to the PoH tick.

## Geyser Plugin
* We need to prove that the blob has been published to solana. This is accomplished by running a geyser plugin inside the solana validator.
* The geyser plugin tracks account updates as blocks are executed and merkle proofs are generated against the `accounts_delta_hash`
* The proofs generated for a Pubkey being monitored are of the form
```rust
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum AccountDeltaProof {
    /// Simplest proof for inclusion in the account delta hash
    InclusionProof(Pubkey, (Data, Proof)),
    /// Adjacency proof for non inclusion A C D E, non-inclusion for B means providing A and C
    NonInclusionProofInner(Pubkey, ((Data, Proof), (Data, Proof))),
    /// Left most leaf and proof
    NonInclusionProofLeft(Pubkey, (Data, Proof)),
    /// Right most leaf and proof. Also need to include hashes of all leaves to verify tree size
    NonInclusionProofRight(Pubkey, (Data, Proof, Vec<Hash>)),
}
```
* The code exists for `InclusionProof`, as well as the `NonInclusionProof`s, but only the inclusion is verified currently.

### Running the Geyser Plugin
* Build the geyser plugin - this is a `.dylib` (or `.so`) that implements the plugin interface and runs inside the solana validator
```bash
cd adapters/solana/account_proof_geyser
cargo build --release
```
* The plugin needs to be built with the same rust version used to build the solana validator. We have a `rust-toolchain.toml` pinning the rust version
* The dynamic lib should be found in `target/release`
```
ls -lahtr target/release/libaccount*
-rw-r--r--  1 username  staff   422B Oct 22 05:58 target/release/libaccount_proof_geyser.d
-rwxr-xr-x  1 username  staff   3.6M Oct 24 05:19 target/release/libaccount_proof_geyser.dylib
-rw-r--r--  1 username  staff    12M Oct 24 05:19 target/release/libaccount_proof_geyser.rlib
```
* The file we care about is `target/release/libaccount_proof_geyser.dylib`
* Build the solana test validator
```bash
git clone git@github.com:solana-labs/solana.git
git checkout tags/v1.16.15
./cargo build --release --bin solana-test-validator
```
* Update `adapater/solana/config.json`
```json
{
    "libpath": "~/sovereign/adapters/solana/account_proof_geyser/target/release/libaccount_proof_geyser.dylib",
    "bind_address": "127.0.0.1:10000",
    "account_list": ["SysvarS1otHashes111111111111111111111111111"]
}
```
 * Change libpath to point to the full path for `libaccount_proof_geyser.dylib`
 * We can leave `account_list` as `SysvarS1otHashes111111111111111111111111111` for now because this is just an example and WIP
* Run the validator with the geyser config
```bash
~/solana/target/release/solana-test-validator --geyser-plugin-config config.json
```
* Once the validator starts up, you can run the tcp client to fetch the Inclusion proofs for `SysvarS1otHashes111111111111111111111111111` each block
```bash
cd adapters/solana/da_client/
cargo run --release --bin simple_tcp_client
    Finished dev [unoptimized + debuginfo] target(s) in 2.36s
     Running `target/debug/simple_tcp_client`
Proof verification succeeded for slot 36172
Proof verification succeeded for slot 36173
Proof verification succeeded for slot 36174
Proof verification succeeded for slot 36175
```

## Work Remaining
* Rigorous testing for merkle proof generation
* Testing for account update processing
  * Currently, the plugin monitors updates as they arrive, moves them to different hashmaps based on SLot updates for "processed" and "confirmed"
  * This works locally, but production validators fork a lot before confirmation, so we need to test this under load to ensure that we're generating proofs correctly
* Test cases for non inclusion proofs (Inclusion has some tests but Non inclusion doesn't)
* `verify_leaves_against_bankhash` needs to updated for Non inclusion proofs using the adjacency checks `are_adjacent` and `is_first`
  * `is_last` is particularly interesting since proving that a leaf is the first leaf is trivial, but proving last leaf is more complicated since we don't have a commitment to the number of leaves in the tree (i.e. the number of accounts updated)
* The `da_client` PDA needs to be plugged into the `simple_tcp_client` as well as the geyser plugin. This would require non inclusion proofs to work
* Currently, the geyser plugin has a simple tcp server that does only one thing - stream account deltas and their inclusion or non-inclusion proofs. We need to replace this with a more comprehensive GRPC server