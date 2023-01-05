### Assumptions

We assume the existence of `c` well-coordinated entities with access to large numbers of GPUs. These parties could be decentralized networks like LayerZ, or enterprises like Supranational. These entites can prove computation at some rate `r`. Ideally, the total throughput
of the network would be `c * r`. Is this achievable?

### Model

Suppose that the network consists of a stream of transactions, which are valid except (possibly) for their ordering. At each time step,
a new blob of 0 or more transactions is added to the network.

Consider block number 10:
At this point, blocks 1-3 have already been proven. Prover A has a proof of block 4 ready to post. Meanwhile, Prover B can work on 5, C can work on 6 etc.
The sequencer will be incentivized to post proof 4. To make this model work, we want the proof of 5 to be able to get started without 4 being available.

```rust
/// A data availability layer
trait Da {
	type BlockHash;
	type Header;
}
/// A state transition function
trait Stf {
	type Block;
	type StateRoot;
	fn apply_block(blk: Block, prev_state: StateRoot) -> StateRoot;
}

trait ChainProof: Proof {
	type DaLayer: Da;
	type Rollup: Stf;
	// returns the hash of the latest DA block
	fn da_hash() -> Da::Blockhash
	// returns the rollup state root
	fn state_root() -> Rollup::StateRoot
}

trait ExecutionProof: Proof {
	type Rollup: Stf;
	fn blocks_applied() -> &[Rollup::Block]
	fn pre_state_root() -> Rollup::StateRoot
	// returns the state root after applying
	fn post_state_root() -> Rollup::StateRoot
}
trait Chain {
	type DaLayer: Da;
	type Rollup: Stf;
	// Verifies that prev_head.da_hash is Da_header.prev_hash
	// calculates the set of sequencers, potentially using the previous rollup state
	// and returns an array of rollup blocks from those sequencers
	fn extend_da_chain(prev_head: ChainProof, header: Da::Header, rollup_namespace_data: &[Da::InclusionProof<Bytes>]) ->  &[Rollup::Block]{
		prev_head.verify()
		assert_eq!(prev_head.last_da_hash(), header.hash())
		let mut blocks = Vec::new();
		// -snip-
		return blocks
	}

	fn apply_rollup_blocks(blks: &[Rollup::Block], prev_root: Rollup::StateRoot) -> Rollup::StateRoot {
		let mut root = prev_head.state_root;
		for blk in rollup_blocks {
			root = Rollup::apply_block(blk, root)
		}
		return root
	}

	fn execute_or_verify_stf(blks: &[Rollup::Block], prev_root: Rollup::StateRoot) -> Rollup::StateRoot {
		let pf: Option<ExecutionProof> = env::read();
		if let Some(proof) = pf {
			assert_eq!(proof.blocks_applied(), blks)
			assert_eq!(proof.pre_state_root(), prev_root)
			proof.verify();
			return proof.state_root()
		}
		return apply_rollup_blocks(blks)
	}

	///
	///
	#[risc0::e]
	fn extend_chain(prev_head: ChainProof<DaLayer = Self::DaLayer, Rollup = Self::Rollup>, da_header: Da::Header, rollup_namespace_data: Array<Da::InclusionProof<Bytes>>) {
		prev_head.verify()

		let rollup_blocks = extend_da_chain(prev_head, header, rollup_namespace_data)
		let output_root =  execute_or_verify_stf(apply_rollup_blocks, prev_head.verify())
		return Proof::new(da_header.hash(), output_root)
	}
}

```
