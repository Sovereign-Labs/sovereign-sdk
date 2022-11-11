// use core::fmt::Debug;

// use crate::StateCommitment;

// /// A state transition function
// pub trait Stf {
//     type Block: PartialEq + Debug;
//     type StateRoot: StateCommitment;
//     type Bundle: Bundle;
//     type Misbehavior;
//     type Error;

//     /// Deserialize a valid bundle into a block. Accept an optional proof of misbehavior (for example, an invalid signature)
//     /// to short-circuit the block application, returning a new stateroot to account for the slashing of the sequencer
//     fn prepare_block(
//         blob: Self::Bundle,
//         prev_state: &Self::StateRoot,
//         misbehavior_hint: Option<Self::Misbehavior>,
//     ) -> Result<Self::Block, Self::StateRoot>;

//     /// Apply a block
//     fn apply_block(blk: Self::Block, prev_state: &Self::StateRoot) -> Self::StateRoot;
// }
