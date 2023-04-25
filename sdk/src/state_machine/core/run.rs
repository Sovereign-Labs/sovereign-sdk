// use bytes::Buf;

// use crate::{
//     core::traits::{BlockHeaderTrait, CanonicalHash},
//     da::{BlobTransactionTrait, DaLayerTrait},
//     serial::DecodeBorrowed,
//     state_machine::env,
//     stf::{ConsensusMessage, StateTransitionFunction},
//     zk::traits::{ProofTrait, RecursiveProofInput, ZkVm},
// };

// use super::types::RollupHeader;

// pub struct Rollup<DaLayer: DaLayerTrait, App: StateTransitionFunction> {
//     pub da_layer: DaLayer,
//     pub app: App,
// }

// pub struct Config<DaLayer: DaLayerTrait, App: StateTransitionFunction> {
//     /// The hash of the DA block which is considered "genesis" for this blockchain.
//     /// Note that this block is *not* necessarily the genesis block of the DA layer. Rather,
//     /// it's the hash of the first DA block which is allowed to contain rollup blocks.
//     pub da_hash_at_rollup_genesis: DaLayer::SlotHash,
//     /// The height after *rollup* genesis at which the chain will start accepting transactions.
//     ///
//     /// This setting is to aid in setting of the genesis block. We have a period of blocks
//     /// after the genesis block which are forced to be "empty" and can be safely ignored.
//     /// This way, we can set the `da_hash_at_genesis` to be a block from (say) yesterday,
//     /// and be sure that the rollup won't actually start processing transactions until (say) next week,
//     /// giving us some time to distribute the code before the rollup goes live.
//     pub first_allowed_nonempty_block_number: u64,

//     // TODO:
//     phantom: std::marker::PhantomData<App>,
// }

// pub struct BlockProof<Vm: ZkVm, DaLayer: DaLayerTrait, App: StateTransitionFunction> {
//     pub phantom: std::marker::PhantomData<Vm>,
//     // phantomapp: std::marker::PhantomData<App>,
//     // phantomda: std::marker::PhantomData<DaLayer>,
//     pub latest_header: RollupHeader<DaLayer, App>,
//     pub code_commitment: Option<Vm::CodeCommitment>,
// }

// impl<Vm: ZkVm<Proof = Self>, DaLayer: DaLayerTrait, App: StateTransitionFunction> ProofTrait<Vm>
//     for BlockProof<Vm, DaLayer, App>
// {
//     type Output = RollupHeader<DaLayer, App>;

//     fn verify(
//         self,
//         code_commitment: &<Vm as ZkVm>::CodeCommitment,
//     ) -> Result<Self::Output, <Vm as ZkVm>::Error> {
//         Vm::verify(self, code_commitment)
//     }
// }

// impl<DaLayer: DaLayerTrait, App: StateTransitionFunction> Rollup<DaLayer, App> {
//     pub fn zk_verify_block<Vm: ZkVm<Proof = BlockProof<Vm, DaLayer, App>>>(
//         &mut self,
//     ) -> Result<BlockProof<Vm, DaLayer, App>, Vm::Error> {
//         // let prev_proof: RecursiveProof<Vm, DaLayer, App> = env::read_unchecked();
//         let prev_proof: RecursiveProofInput<
//             Vm,
//             RollupHeader<DaLayer, App>,
//             BlockProof<Vm, DaLayer, App>,
//         > = env::read_unchecked();
//         // Three steps:
//         // 1. Validate input (check proof or confirm that the hash is correct)
//         // 2. Tie input to current step
//         // 3. Do work

//         let (prev_header, code_commitment) = match prev_proof {
//             RecursiveProofInput::Base(purported_genesis) => {
//                 assert!(purported_genesis.da_block_hash == DaLayer::RELATIVE_GENESIS);
//                 // TODO! more checks
//                 (purported_genesis, None)
//             }
//             RecursiveProofInput::Recursive(proof, _) => {
//                 let commitment = proof
//                     .code_commitment
//                     .clone()
//                     .unwrap_or_else(env::read_unchecked);
//                 let prev_header = proof.verify(&commitment)?;
//                 (prev_header, Some(commitment))
//             }
//         };
//         let current_da_header: DaLayer::BlockHeader = env::read_unchecked();
//         assert_eq!(&prev_header.da_block_hash, current_da_header.prev_hash());

//         let relevant_txs = env::read_unchecked();
//         let tx_witness = env::read_unchecked();
//         let completeness_proof = env::read_unchecked();
//         self.da_layer
//             .verify_relevant_tx_list(
//                 &current_da_header,
//                 relevant_txs,
//                 tx_witness,
//                 completeness_proof,
//             )
//             .expect("Host must provide correct data");

//         let mut current_sequencers = prev_header.sequencers_root.clone();
//         let mut current_provers = prev_header.provers_root.clone();

//         self.app.begin_slot();
//         for tx in relevant_txs {
//             let mut data = tx.data();
//             let len = data.remaining();
//             let data = data.copy_to_bytes(len);
//             match ConsensusMessage::decode_from_slice(&data[..]).unwrap() {
//                 ConsensusMessage::Batch(batch) => {
//                     if current_sequencers.allows(tx.sender()) {
//                         match self.app.apply_batch(batch, tx.sender().as_ref(), None) {
//                             // TODO: handle events
//                             Ok(_events) => {}
//                             Err(_slashing) => todo!(), //current_sequencers.process_update(slashing),
//                         };
//                     }
//                 }
//                 ConsensusMessage::Proof(p) => {
//                     if current_provers.allows(tx.sender()) {
//                         match self.app.apply_proof(p, tx.sender().as_ref()) {
//                             Ok(()) => {}
//                             Err(slashing) => current_provers.process_update(slashing),
//                         };
//                     }
//                 }
//             }
//         }
//         let (app_hash, consensus_updates) = self.app.end_slot();
//         for update in consensus_updates {
//             if let Some(role) = &update.new_role {
//                 match role {
//                     crate::stf::ConsensusRole::Prover => {
//                         current_provers.process_update(update);
//                         continue;
//                     }
//                     crate::stf::ConsensusRole::Sequencer => {
//                         current_provers.process_update(update);
//                         continue;
//                     }
//                     crate::stf::ConsensusRole::ProverAndSequencer => {}
//                 }
//             }
//             current_provers.process_update(update.clone());
//             current_sequencers.process_update(update);
//         }
//         current_provers.finalize();
//         current_sequencers.finalize();

//         let header = RollupHeader {
//             da_block_hash: current_da_header.hash(),
//             sequencers_root: current_sequencers,
//             provers_root: current_provers,
//             app_root: app_hash,
//             applied_txs_root: Default::default(), // TODO!,
//             prev_hash: prev_header.hash(),
//         };
//         Ok(BlockProof {
//             phantom: std::marker::PhantomData,
//             latest_header: header,
//             code_commitment,
//         })
//     }
//     // fn zk_verify_block<Vm: ZkVM<Proof = RecursiveProof<Vm, BlockProofOutput>>>() {
//     //     let prev_proof: RecursiveProof<Vm, BlockProofOutput> = env::read_unchecked();
//     //     // prev_proof.recurse()
//     // }
// }
