# Bridging

Interoperability is at the core of Stratum's value proposition. To facilitate bridging, we use off-chain proof aggregation. This allows
each rollup to connect to a large number of peers with very low overhead.

## Model

We assume a large ecosystem of validity-proven rollups using Risc0 as the underlying proof system. In this model, a rollup can be uniquely
identified by its `MethodID` - a cryptographic fingerprint of its state transition function. Risc0 proofs consist of a series of some proof
data and a tamper-proof `log` of outputs written by the `guest` during execution.

We further assume that all rollups define their fork choice to be a pure function of the underlying DA layer, and execute that function
in-circuit. The only other constraint that we place on rollups is that the output log of their proofs must start with a sentinal value
corresponding to the discriminant of an enum

```rust
/// A model of a risc0 proof
pub trait Proof {
	get_log() -> &[u8]
	verify(id: MethodId) -> Result<(), VerificationError>
}

pub struct AggregateProofOutput {
	aggregator_id: Option<MethodId>,
	verified_outputs: Vec<(MethodId, RollupProofOutput)>,
}

#[derive(Serialize, Deserialize)]
pub enum ProofOutput {
	Aggregate(AggregateProofOutput),
	/// All rollup proofs must serialize their output log to a Vec<u8>, and must write the discriminant of this
	/// variety to their log before outputting the vector!
	Individual(Vec<u8>),
}
```

## Proof Creation and Verification

We define the following method for proof aggregation

```rust
use risc0_guest::verifable_output_log as log;

/// This function is invoked repeatedly off-chain to create an aggregate proof for all rollups
fn aggregate_proofs(p1: (MethodId, Proof), p2: (MethodId, Proof)) {
	let mut outer_aggregator_id: Option<MethodId> = None;
	let verified_outputs: Vec::new();
	for (method_id, pf) in [p1, p2] {
		pf.verify(method_id).expect("only valid proofs can be aggregated");
		let output = ProofOutput::deserialize(pf.get_log()).expect("valid proofs must have valid output logs");
		match output {
			Aggregate(aggregate_proof) => {
				// When verifying alleged aggregate proofs, we need to verify that the same method has been used as the aggregator.
				// all the way through the recursion stack. Simply matching on the
				// contents of the output log isn't enough, since many programs with different
				// logic could write the same method id - including programs which don't verify their inner proofs.
				if let Some(inner_aggregator_id) = aggregate_proof.aggregator_id {
					assert_eq!(inner_aggregator_id, method_id)
				}
				// If we've already observed an aggreagor ID, check that it matches
				// the method id used to verify this proof
				if let Some(outer_id) = outer_aggregator_id {
					assert_eq!(outer_id, &method_id);
				}
				// If we haven't already observed an aggregator id, record this one
				outer_aggregator_id.get_or_insert(method_id);
				verified_outputs.extend_from_slice(&aggregate_proof.verified_outputs);
			},
			Individual(proof)=> {
				// We don't care what the method ID of an individual proof is - any valid program is allowed
				verified_outputs.push((method_id, proof))
			}
		}
	}
	log!(ProofOutput::Aggregate(AggregateProofOutput {
		aggregator_id: outer_aggregator_id,
		verified_outputs,
	}))
}
```

Meanwhile, the on-chain verifier must implement the following logic

```rust
pub trait AggregateVerifier {
	const AGGREGATOR_METHOD_ID: MethodId;
	fn verify_aggregate_proof(&mut self, aggregate_proof: Proof) -> Vec<(MethodId, RollupProofOutput)> {
		aggregate_proof.verify(Self::AGGREGATOR_METHOD_ID);
		let output =  ProofOutput::deserialize(pf.get_log()).expect("valid proofs must have valid output logs");
		if let ProofOutput::Aggregate(aggregate_output) = output {
			aggregate_output.aggregator_id.map(|id|, assert_eq!(Self::AGGREGATOR_METHOD_ID, id));
			return aggregate_output.verified_outputs
		}
		unreachable!("A valid aggregate proof must create have an AggregateProofOutput!")
}

```
