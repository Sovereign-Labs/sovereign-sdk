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

### Installation and Setup
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

## Running the demo
### Setting up and starting a local solana validator
* Create a folder to hold your solana keys
```bash
$ mkdir ~/.solw
```
* Generate a keypair
```bash
$ solana-keygen new -o ~/.solw/test.json
```
* Set the keypair in the solana cli
  * For now, do not set a passphrase for testing since it complicates the workflow a bit
```
$ solana config set -k ~/.solw/test.json
```
* Point the solana cli to local
```bash
$ solana config set -u l
```
* Check solana config
```
$ solana config get
Config File: /Users/dubbelosix/.config/solana/cli/config.yml
RPC URL: http://localhost:8899 
WebSocket URL: ws://localhost:8900/ (computed)
Keypair Path: ~/.solw/test.json 
Commitment: confirmed 
```
* We also need to build the geyser plugin that streams account updates, so that a listener can calculate 
  * A merkle proof of rollup block existence
  * A non inclusion merkle proof of a rollup block being absent
* Build the geyser plugin
```bash
cd sovereign/adapters/solana/
make
```
* Start the `solana-test-validator` with the plugin config
```bash
$ cd sovereign
$ solana-test-validator --geyser-plugin-config adapters/solana/config.json
Ledger location: test-ledger
Log: test-ledger/validator.log
⠴ Initializing...                                                                                                                
⠉ 00:00:12 | Processed Slot: 25 | Confirmed Slot: 25 | Finalized Slot: 0 | Full Snapshot Slot: - | Incremental Snapshot Slot: - |
```
* Check balance of primary account from config
```bash
$ solana balance
500000000 SOL
```
* Leave this terminal running and in a new window, start the logs monitoring
```bash
$ solana logs
```
* Some notes
  * The `solana-test-validator` creates and uses a `test-ledger` folder in the current working directory. 
  * If killed and restarted, the `solana-test-validator` continues from where it left off. 
  * If the ledger needs to be wiped clean, then this folder can just be removed and the local dev chain will start with a fresh genesis.
  * If the key needs to be changed and you need more local network sol, then you can switch the key using `solana config -k` and request sol using `solana airdrop`

### The on-chain program
* Ensure anchor is installed by following the steps from the [Installation and Setup](#installation-and-setup) section
* Build and deploy the on-chain program
```bash
$ cd solana/solana_da_programs
$ anchor build --program-name blockroot 
$ anchor deploy --program-name blockroot --provider.wallet ~/.solw/test.json
Deploying cluster: http://localhost:8899
Upgrade authority: /Users/dubbelosix/.solw/test.json
Deploying program "blockroot"...
Program path: ~/sovereign/adapters/solana/solana_da_programs/target/deploy/blockroot.so...
Program Id: 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E
```
* Once the program is deployed, we need the Program ID from the output for the next steps `6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E`

### The solana da client
* The client is written in rust (there are SDKs in typescript, python, go etc) 
* Rust makes it easier since we can share structs, serde and functions directly between on-chain program and client
* The client actually makes use of a lot of the function from `solana_da_programs/programs/blockroot`
* Build and run the client
```
$ cd solana/da_client
$ cargo run -- -h
Usage: da_client [OPTIONS] --signer <SIGNER> --blockroot-program <BLOCKROOT_PROGRAM> <COMMAND>

Commands:
  chunk-account     Manage the chunks account on chain. This is the scratch space for accumulating chunks on chain scoped to a sequencer
  create-test-data  Produce test data (Random bytes of desired size)
  submit            Submit chunks to the chain
  verify            
  help              Print this message or the help of the given subcommand(s)

Options:
      --signer <SIGNER>
          Path to the signer key
      --blockroot-program <BLOCKROOT_PROGRAM>
          b58 encoded address for the on chain sovereign blockroot program
  -r, --rpc-url <RPC_URL>
          URL for solana RPC [default: http://localhost:8899]
  -w, --ws-url <WS_URL>
          URL for solana Websocket [default: ws://localhost:8900]
  -h, --help
          Print help
  -V, --version
          Print version
```
* We first want to create our chunk-account
```bash
da_client $ cargo run -- --signer ~/.solw/test.json --blockroot-program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E chunk-account create ~/.solw/chunk_account.json
```
  * `--blockroot-program` is the on-chainprogram that we deployed
  * `--signer` is the key that can sign transactions and pay for account storage
  * `~/.solw/chunk_account.json` is where we want the keypair chunk account to be created. Explanation of what the chunk account is used for is provided in the next section
* Create a test blob of 100kb (Very large sizes cause the solana program to run OOM. We need to test these limits and see if we can go higher)
```bash
$ cargo run -- --signer ~/.solw/test.json --blockroot-program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E create-test-data testblob 100000
```
* The above command should create a file called `testblob` in the current working directory
* We now need to submit blob (which represents a block of rollup transactions) to solana
```bash
da_client $ cargo run -- --signer ~/.solw/test.json --blockroot-program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E submit ~/.solw/chunk_account.json testblob 
raw data file: testblob
digest: 50267856fe52ceb63b95b4bb70f203952bf18d1ac141b5d886dc9a6feb4d09b8
number of chunk transactions: 131
chunks digest for blob file at testblob is 50267856fe52ceb63b95b4bb70f203952bf18d1ac141b5d886dc9a6feb4d09b8 
Ok(5UWtJPavtr2NjRN1gxsxKrtrPoQNB9Jeq6CF3Ya8GFxeS7BHruscAjpYrHW8ALd3QAdEyDnXsnR4Wo5jEmfecepe)
Ok(67ZBFYjXiqYrVXqNqEByFjmvZVp67dzWuyJUJtqTkhNpask1mxPQGECCseBNEPu8MkXwCZJtF11v8HhhhapaXTPB)
Ok(2mentFby7BnW64NuM7EQBgfpsUmYU37iffwDq8vLj2xrCVxfNitUkK1iL73rWN2UwJ8bqR1HNBBorThqWctShbdf)
Ok(2GX1fTkufWZJVn88TiMbBEkn46NAc8acqguVAbVMhowZiJEZvNppJVWi2opC4xVoT31pcqhiaCKSjS51bNwF8AyW)
.
.
.
Ok(44CoVzXpXyYKS8ucjsebuaRjwpf6bvuEzEPpaXNY85LUbT62kLc58Z21TJ6ceLfE2XU9GJevEw5k4okMaRGd1rTV)
```
* Simultaneously, you can see the chunk accumulation working on the terminal that's running `solana logs`
```bash
Transaction executed in slot 9290:
  Signature: 3bgUwqNPvKCG3yz5AXCAVhg9TbBBJqtbzsurJUXRzDLb4UMGT7d3T2pgC1kBQyT1dFmVbXfVT22dn8Uv3vJUPidZ
  Status: Ok
  Log Messages:
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E invoke [1]
    Program log: Instruction: ProcessChunk
    Program log: false
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E consumed 60998 of 200000 compute units
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E success
Transaction executed in slot 9291:
  Signature: 2coXd479bV8H1fWhq43WYr9Ay1K1aezHhHL4h9hfNSjuxLW8F6Y6kJPF1kNBDJfKkKo2jQ4eVhwEJGVoH3jk9X1X
  Status: Ok
  Log Messages:
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E invoke [1]
    Program log: Instruction: ProcessChunk
    Program log: false
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E consumed 60876 of 200000 compute units
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E success
```
* The important lines are under `Log Messages`
```bash
    Program 6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E invoke [1]
    Program log: Instruction: ProcessChunk
    Program log: false
```
* The "false" indicates that the data hasn't been completely accumulated yet. Once the last chunk transaction is submitted, we should see
```bash
    Program log: Instruction: ProcessChunk
    Program log: true
    Program log: accumulation blob with digest: [80, 38, 120, 86, 254, 82, 206, 182, 59, 149, 180, 187, 112, 242, 3, 149, 43, 241, 141, 26, 193, 65, 181, 216, 134, 220, 154, 111, 235, 77, 9, 184] has completed with root [80, 38, 120, 86, 254, 82, 206, 182, 59, 149, 180, 187, 112, 242, 3, 149, 43, 241, 141, 26, 193, 65, 181, 216, 134, 220, 154, 111, 235, 77, 9, 184]
    Program log: blocks root for slot 9431, blob root: [80, 38, 120, 86, 254, 82, 206, 182, 59, 149, 180, 187, 112, 242, 3, 149, 43, 241, 141, 26, 193, 65, 181, 216, 134, 220, 154, 111, 235, 77, 9, 184] combined root: [80, 38, 120, 86, 254, 82, 206, 182, 59, 149, 180, 187, 112, 242, 3, 149, 43, 241, 141, 26, 193, 65, 181, 216, 134, 220, 154, 111, 235, 77, 9, 184]
```
* The `true` indicates that accumulation as completed and the merkle root for the chunks is also logged as having been written to another account

### Explanation of the Process
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
    * `actual_size`: The chunks are equal sized, so the final chunk has padding. `actual_size` is used to enable stripping out padding during reconstruction.
      * We can do away with padding if we find that it's un-necessary.
* The `blockroot` program contains 3 instructions
  * Initialize - used to initialize the accounts
  * Clear - Used to clear the `ChunkAccumulator` account of any incomplete blobs.
    * This is only necessary if there are incomplete chunks
    * Ultimately, management of the "scratch space" is up to the sequencers, and they can make decisions around size, cleanup etc
  * ProcessChunk - logic to keep track of the chunks as they arrive and calculate the merkle root of the chunks
* Each Chunk is formatted into a transaction for the `ProcessChunk` instruction.
* `ProcessChunk` works with 2 main accounts
  * `ChunkAccumulator` account - this account is owned by the sequencer, has a keypair and can only be modified by the `blockroot` program since it's the owner of the account
  * `BlocksRoot` account - this is a PDA (program derived address) account that is unique to the entire program
* When the `ProcessChunk` instruction first sees a new `digest` indicating a new blob, it extracts the necessary metadata from the chunk and allocates space for a merkle tree capable of handling `num_chunks` number of leaves in the `ChunkAccumulator` account
* The new merkle tree has empty slots and `Chunk`s are inserted into the right positions as they arrive
* The `ProcessChunk` instruction also bubbles up the hashes towards the root as the chunks arrive
  * This has the benefit of spreading compute among multiple transactions
  * Each transaction performs a fraction of work in calculating the merkle root
  * The specific logic for merkelization can be seen in the `accumulate` function in [lib.rs](solana_da_programs/programs/blockroot/src/lib.rs)
  * The code also makes use of the `hashv` syscall.
  * For odd numbered levels, the un-paired leaves are promoted to the next level. (This logic needs to be audited since its custom)
```
          Root
           |
     +-----+----------+
     |                |
  Parent1         Parent2 [X]
     |                |
  +--+--+       +-----+------+
  |     |       |            |
Chunk1 Chunk2  Chunk3     Chunk4
 [ ]   [ ]      [X]         [X]

```
  * The above diagram illustrates what the `ChunkAccumulator` might look like at a point in time when `Chunk3` and `Chunk4` have arrived
  * `Chunk3` arrives first and everything else in the tree is empty.
  * `Parent2` is calculated once `Chunk4` arrives
  * `Chunk1`, `Chunk2`, `Parent1` and `Root` are still incomplete
* Chunks can arrive in any order since they get inserted into the right slots of the tree in the `ChunkAccumulator` account's on-chain space
* Once the final chunk arrives and the merkle root is available, the root is "accumulated" into the `BlocksRoot` PDA account
  * This accumulation takes the form of a hashlist (since we don't really need merkelization here)
  * The `BlocksRoot` account also keeps track of the current slot number
  * If multiple chunks arrive and finish multiple blobs in a single slot, then all their merkle roots will be written into the blocksroot account
  * As an example, assume 4 final chunks arrive and 4 merkle roots are available (M1, M2, M3, M4) at slot 4200
    * The first merkle root to finish, M1, observes that the slot number is < 4200, so it's the first root.
    * The digest is set to M1 and slot_number is changed to 4200 (digest = M1)
    * The second merkle root, M2 observes that the slot_number in the account is equal to the current slot number, so its not the first blob to finish
    * Therefore, it accumulates itself into the digest (digest = hashv(digest|M2)) which is just hashv(M1|M2)
    * By the time the final root is accumulated the Digest would be hashv(hashv(hashv(M1|M2)|M3)|M4)
* A solana Bank Hash stores the account delta hash in each block. Since 4 blob roots were accumulated in slot 4200, this means that the account value for `BlocksRoot` changed, so it's hash would be part of the Bank Hash
* The Solana Bank hash providing a commitment to the value of `BlocksRoot` account is ultimately what enables DA
* The accounts delta root is also calculated by sorting the leaves, which means we can also prove non-inclusion
  * Showing that the `BlocksRoot` PDA doesn't exist between two adjacent leaves tells us that there are no rollup blobs in a specific Solana block

### Block Processor
* Run the block processor binary, which streams geyser updates from the `solana-test-validator`
```bash
cd sovereign/adapters/solana/da_client
cargo run --bin account_delta_processor
```
* The above should print a stream of slots, modified accounts and their hashes
```
slot:30638, pubkey:"SysvarS1otHashes111111111111111111111111111", hash:"E4NFvB38MnE5LPPPcQB5LUfoMGMhKDMiZs4mQzJ8qCcG"
slot:30638, pubkey:"SysvarC1ock11111111111111111111111111111111", hash:"EUgR9BNdKjrnwZTMqwJ2CD3fdTkRn82axLAeCokx7NK5"
slot:30638, pubkey:"5Pmg4aiLg3WYzns5FCpbs5bxxNRzdb6YwKJemykPbZBD", hash:"G8LNAUrjhdGhX5npczHJT8j2hR5qTvvKnKnvZS4jesgD"
slot:30638, pubkey:"SysvarRecentB1ockHashes11111111111111111111", hash:"2d3xmQ1ueKR3Jk2cQsN9uEFsMfjDZmohHESpHz3MBfiC"
slot:30638, pubkey:"5MgKRwYGsa9S7Shtch7UAjXhzGUaze5u2dx1NuP4oskH", hash:"txi5QmUUCbXMkcD5CPTpyin3X4Ky1gvRKGyzJpc2UB5"
slot:30638, pubkey:"SysvarS1otHistory11111111111111111111111111", hash:"GSGN4DcEBN49rFaMLDCTmMkXuh1yKfT4z9R4mZJHFD34"
slot:30639, pubkey:"SysvarS1otHashes111111111111111111111111111", hash:"Dmc2WP9psnKG476ahjwTHgnFaimpXNSkJQcTG7zoYTXQ"
slot:30639, pubkey:"SysvarC1ock11111111111111111111111111111111", hash:"GZev34FK7jn5X7Du3joegLb4YuKxaMAByy7JcVXzj3w3"
slot:30639, pubkey:"5Pmg4aiLg3WYzns5FCpbs5bxxNRzdb6YwKJemykPbZBD", hash:"2rLWZfTBhpiHWmg8n3w737By9Bcvs6N4zbRKZZaN2KDB"
slot:30639, pubkey:"SysvarRecentB1ockHashes11111111111111111111", hash:"C5gqC6mZdDH3Lr33GLusDstSqX4ftTbWd34nTrgBppnh"
slot:30639, pubkey:"5MgKRwYGsa9S7Shtch7UAjXhzGUaze5u2dx1NuP4oskH", hash:"AYVKX9n4Gs9zRLpW4Nsu5o68k7ijE2VitDLT2HAwi8jz"
```

### TBD
* The logic to fetch the bank hash and verify availability still needs to be written into the da_client
* Test cases for merkelization

### Notes
* Since `ChunkAccumulator` is unique to each sequencer, multiple sequencers can do this simultaneously
* A single sequencer can also have a pool of accounts that it uses for this purpose
* The sol paid for rent exemption (approximate 69 sol for 10MB of on-chain space) can be reclaimed by closing down the account if needed
