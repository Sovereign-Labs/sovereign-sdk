# zkVM

The sovereign SDK is designed to support any zkVM capable of running Rust code.
However, VMs must be capable of supporting a standard set of APIs.

## A Note on Performance

This specification does *not* define any standards relating to performance: proof size, prover work,
verification time, latency. This omission should not be understood to imply
that the SDK will work equally well for all choice of proof system. However, the SDK will *function* correctly when
defined in any sound proof system, we don't define any specific requirements.
We strongly suggest that users consider a performant VM such as Risc0.

## Methods

### Log

* **Usage:**
  * The `log` method adds an item to the proof's public output. These outputs are committed
  to in the proof, so that any tampering with the output will cause proof verification
  to fail.

* **Arguments**
  | name | type | description |
  |------|------|-------------|
  | item | impl Serialize | The item to be appended to the output. May be any struct that supports serialization |

### Verify

* **Usage:**
  * Verifies a zero-knowledge proof, including all of its public outputs

* **Arguments**
  | name | type | description |
  |------|------|-------------|
  | proof | PROOF | A zero-knowledge proof, including its public outputs |
  | code_commitment | CODE_IDENT | A cryptographic commitment identifying the program which produced this proof |

* **Response**
  | name | type | description |
  |------|------|-------------|
  | Ok | any | The deserialized contents of the proof's public outputs |
  | Err | ERROR | An VM-defined error type |
  * Note: This is a `Result` type. only one of the `Ok` and `Err` fields will be populated.

## Structs

### Proof

A proof is a VM-defined type. It must support serialization, deserialization, and
verification - but it is otherwise opaque to the SDK.

### Error

The zkVM MUST also define an Error type which SHOULD convey useful information to the caller
when proof verification fails.

## Example Code

Expressed in Rust, zkVM would be a `trait` that looked something like the following:

```rust
pub trait Zkvm {
  type CodeCommitment: PartialEq + Clone;
  type Proof: Encode + Decode<Error=DeserializationError>;
  type Error;

  fn log<T: Encode>(item: T);
  fn verify<T: Decode>(
    proof: Self::Proof,
    code_commitment: &Self::CodeCommitment,
  ) -> Result<T, Self::Error>;
}
```
