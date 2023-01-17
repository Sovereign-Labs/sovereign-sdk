use crate::{
    core::traits::{BlockTrait, BlockheaderTrait, CanonicalHash},
    core::types::RollupHeader,
    da::{DaApp, BlobTransactionTrait},
    state_machine::env,
    stf::StateTransitionFunction,
    zk::traits::{ProofTrait, RecursiveProofInput, ZkVM},
};

use super::run::BlockProof;

pub struct Rollup<DaLayer: DaApp, App: StateTransitionFunction> {
    pub da_layer: DaLayer,
    pub app: App,
}

// pub struct Runner<
//     DaProvider: crate::da::DaService,
//     Db,
//     DaLogic: DaApp,
//     App: StateTransitionFunction,
// > {
//     pub rollup: Rollup<DaLogic, App>,
//     pub da_service: DaProvider,
//     pub db: Db,
// }

// impl<DaProvider: crate::da::DaService, Db, DaLogic: DaApp, App: StateTransitionFunction>
//     Runner<DaProvider, Db, DaLogic, App>
// {
// }

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
