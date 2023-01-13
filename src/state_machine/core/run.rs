use crate::{
    core::traits::{Block, Blockheader, CanonicalHash},
    da::{DaApp, BlobTransaction},
    state_machine::env,
    stf::StateTransitionFunction,
    zk::traits::{Proof, RecursiveProofInput, ZkVM},
};

use super::types::RollupHeader;

pub struct Rollup<DaLayer: DaApp, App: StateTransitionFunction> {
    pub da_layer: DaLayer,
    pub app: App,
}

pub struct Config<DaLayer: DaApp, App: StateTransitionFunction> {
    /// The hash of the DA block which is considered "genesis" for this blockchain.
    /// Note that this block is *not* necessarily the genesis block of the DA layer. Rather,
    /// it's the hash of the first DA block which is allowed to contain rollup blocks.
    pub da_hash_at_rollup_genesis: DaLayer::Blockhash,
    /// The height after *rollup* genesis at which the chain will start accepting transactions.
    ///
    /// This setting is to aid in setting of the genesis block. We have a period of blocks
    /// after the genesis block which are forced to be "empty" and can be safely ignored.
    /// This way, we can set the `da_hash_at_genesis` to be a block from (say) yesterday,
    /// and be sure that the rollup won't actually start processing transactions until (say) next week,
    /// giving us some time to distribute the code before the rollup goes live.
    pub first_allowed_nonempty_block_number: u64,

    // TODO:
    phantom: std::marker::PhantomData<App>,
}

pub struct BlockProof<Vm: ZkVM, DaLayer: DaApp, App: StateTransitionFunction> {
    pub phantom: std::marker::PhantomData<Vm>,
    // phantomapp: std::marker::PhantomData<App>,
    // phantomda: std::marker::PhantomData<DaLayer>,
    pub latest_header: RollupHeader<DaLayer, App>,
    pub code_commitment: Option<Vm::CodeCommitment>,
}

impl<Vm: ZkVM<Proof = Self>, DaLayer: DaApp, App: StateTransitionFunction> Proof<Vm>
    for BlockProof<Vm, DaLayer, App>
{
    type Output = RollupHeader<DaLayer, App>;

    fn verify(
        self,
        code_commitment: &<Vm as ZkVM>::CodeCommitment,
    ) -> Result<Self::Output, <Vm as ZkVM>::Error> {
        Vm::verify(self, code_commitment)
    }
}

impl<DaLayer: DaApp, App: StateTransitionFunction> Rollup<DaLayer, App> {
    pub fn zk_verify_block<Vm: ZkVM<Proof = BlockProof<Vm, DaLayer, App>>>(
        &mut self,
    ) -> Result<BlockProof<Vm, DaLayer, App>, Vm::Error> {
        // let prev_proof: RecursiveProof<Vm, DaLayer, App> = env::read_unchecked();
        let prev_proof: RecursiveProofInput<
            Vm,
            RollupHeader<DaLayer, App>,
            BlockProof<Vm, DaLayer, App>,
        > = env::read_unchecked();
        // Three steps:
        // 1. Validate input (check proof or confirm that the hash is correct)
        // 2. Tie input to current step
        // 3. Do work

        let (prev_header, code_commitment) = match prev_proof {
            RecursiveProofInput::Base(purported_genesis) => {
                assert!(purported_genesis.da_blockhash == DaLayer::RELATIVE_GENESIS);
                // TODO! more checks
                (purported_genesis, None)
            }
            RecursiveProofInput::Recursive(proof, _) => {
                let commitment = proof
                    .code_commitment
                    .clone()
                    .unwrap_or_else(|| env::read_unchecked());
                let prev_header = proof.verify(&commitment)?;
                (prev_header, Some(commitment))
            }
        };
        let current_da_header: DaLayer::BlockHeader = env::read_unchecked();
        assert_eq!(&prev_header.da_blockhash, current_da_header.prev_hash());

        let relevant_txs = env::read_unchecked();
        let tx_witness = env::read_unchecked();
        let completeness_proof = env::read_unchecked();
        self.da_layer
            .verify_relevant_tx_list(
                &current_da_header,
                relevant_txs,
                tx_witness,
                completeness_proof,
            )
            .expect("Host must provide correct data");

        let mut current_sequencers = prev_header.sequencers_root.clone();
        let mut current_provers = prev_header.provers_root.clone();

        self.app.begin_slot();
        for tx in relevant_txs {
            if current_sequencers.allows(tx.sender()) {
                match self.app.parse_block(tx.data(), tx.sender().as_ref()) {
                    Ok(block) => {
                        if let Err(slashing) = self.app.begin_block(
                            &block,
                            tx.sender().as_ref(),
                            env::read_unchecked(),
                        ) {
                            current_sequencers.process_update(slashing);
                            continue;
                        }
                        for tx in block.take_transactions() {
                            self.app.deliver_tx(tx);
                        }
                        let result = self.app.end_block();
                        current_provers.process_updates(result.prover_updates);
                        current_sequencers.process_updates(result.sequencer_updates);
                    }
                    Err(slashing) => slashing
                        .into_iter()
                        .for_each(|update| current_sequencers.process_update(update)),
                }
            } else if current_provers.allows(tx.sender()) {
                match self.app.parse_proof(tx.data(), tx.sender().as_ref()) {
                    Ok(proof) => {
                        if let Err(slashing) = self.app.deliver_proof(proof, tx.sender().as_ref()) {
                            slashing
                                .into_iter()
                                .for_each(|update| current_provers.process_update(update));
                        }
                    }
                    Err(slashing) => slashing
                        .into_iter()
                        .for_each(|update| current_provers.process_update(update)),
                }
            }
        }
        let app_hash = self.app.end_slot();
        current_provers.finalize();
        current_sequencers.finalize();

        let header = RollupHeader {
            da_blockhash: current_da_header.hash(),
            sequencers_root: current_sequencers,
            provers_root: current_provers,
            app_root: app_hash,
            applied_txs_root: Default::default(), // TODO!,
            prev_hash: prev_header.hash(),
        };
        Ok(BlockProof {
            phantom: std::marker::PhantomData,
            latest_header: header,
            code_commitment,
        })
    }
    // fn zk_verify_block<Vm: ZkVM<Proof = RecursiveProof<Vm, BlockProofOutput>>>() {
    //     let prev_proof: RecursiveProof<Vm, BlockProofOutput> = env::read_unchecked();
    //     // prev_proof.recurse()
    // }
}
