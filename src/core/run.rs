use crate::{
    core::traits::{Address, Block, Blockheader},
    da::{DaApp, TxWithSender},
    env,
    stf::StateTransitionFunction,
    zk_utils::traits::{Proof, RecursiveProof, RecursiveProofInput, ZkVM},
};

use super::{crypto::hash::DefaultHash, types::RollupHeader};

pub struct Rollup<DaLayer: DaApp, App: StateTransitionFunction> {
    da_layer: DaLayer,
    app: App,
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
    phantom: std::marker::PhantomData<Vm>,
    // phantomapp: std::marker::PhantomData<App>,
    // phantomda: std::marker::PhantomData<DaLayer>,
    pub da_genesis_hash: DaLayer::Blockhash,
    pub latest_header: RollupHeader<DaLayer, App>,
    pub code_commitment: Vm::CodeCommitment,
}

impl<Vm: ZkVM, DaLayer: DaApp, App: StateTransitionFunction> Proof<Vm>
    for BlockProof<Vm, DaLayer, App>
{
    type Output = RollupHeader<DaLayer, App>;

    fn verify(
        self,
        code_commitment: &<Vm as ZkVM>::CodeCommitment,
    ) -> Result<Self::Output, <Vm as ZkVM>::Error> {
        todo!()
    }
}

impl<Vm: ZkVM, DaLayer: DaApp, App: StateTransitionFunction> RecursiveProof
    for BlockProof<Vm, DaLayer, App>
{
    type Vm = Vm;

    type InOut = RollupHeader<DaLayer, App>;
    type Error = Vm::Error;

    fn verify_base(input: Self::InOut) -> Result<(), Self::Error> {
        todo!()
    }

    fn verify_continuity(previous: Self::InOut) -> Result<(), Self::Error> {
        todo!()
        // if previous.latest_header.da_blockhash == self.latest_header. {
        // 	return Ok(())
        // }
        // Err(())
    }

    fn work(
        input: Self::InOut,
    ) -> crate::zk_utils::traits::RecursiveProofOutput<Self::Vm, Self::InOut> {
        todo!()
    }
}
impl<DaLayer: DaApp, App: StateTransitionFunction> Rollup<DaLayer, App> {
    fn zk_verify_block<Vm: ZkVM>(&mut self) -> Result<BlockProof<Vm, DaLayer, App>, Vm::Error> {
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

        let prev_header = match prev_proof {
            RecursiveProofInput::Base(purported_genesis) => {
                assert!(purported_genesis.da_blockhash == DaLayer::RELATIVE_GENESIS);
                // TODO! more checks
                purported_genesis
            }
            RecursiveProofInput::Recursive(proof, _) => {
                let commitment = proof.code_commitment.clone();
                let prev_header = proof.verify(&commitment)?;
                prev_header
            }
        };
        let current_da_header: DaLayer::Header = env::read_unchecked();
        assert_eq!(&prev_header.da_blockhash, current_da_header.prev_hash());

        let relevant_txs = env::read_unchecked();
        let tx_witness = env::read_unchecked();
        let completeness_proof = env::read_unchecked();
        self.da_layer
            .verify_relevant_tx_list(
                &current_da_header,
                &relevant_txs,
                &tx_witness,
                completeness_proof,
            )
            .expect("Host must provide correct data");

        let current_sequencers = prev_header.sequencers_root;
        let current_provers = prev_header.provers_root;

        self.app.begin_slot();
        for tx in relevant_txs {
            if current_sequencers.allows(tx.sender()) {
                if let Ok(block) = self.app.parse_block(tx.data(), tx.sender().as_bytes()) {
                    self.app.begin_block(block.header());
                    for tx in block.take_transactions() {
                        self.app.deliver_tx(tx);
                    }
                    let result = self.app.end_block();
                    current_provers.process_updates(result.prover_updates)
                }
            } else if current_provers.allows(tx.sender()) {
                if let Ok(block) = self.app.parse_block(tx.data(), tx.sender().as_bytes()) {
                    // self.app.begin_block(block.header());
                }
            }
        }
        // let current_header: RollupHeader<DaLayer, App> = env::read_unchecked();
        // assert_eq!(current_da_header.hash(), &current_header.da_blockhash);

        todo!()

        // prev_proof.recurse()
    }
    // fn zk_verify_block<Vm: ZkVM<Proof = RecursiveProof<Vm, BlockProofOutput>>>() {
    //     let prev_proof: RecursiveProof<Vm, BlockProofOutput> = env::read_unchecked();
    //     // prev_proof.recurse()
    // }
}
