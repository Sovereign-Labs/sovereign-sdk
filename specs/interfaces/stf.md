# State transition Function

## Overview

The state transition function is the "business logic" of a rollup.
It is defined in a particular zkVM (since implementation details like hash and proof formats functions need to change). However,
an implementation of an STF may be reused across multiple DA layers.

The Sovereign SDK guarantees that all relevant transactions will be delivered to the STF for processing
exactly once, in the order that they appear on the DA layer. The STF is responsible for implementing its own metering
and billing (to prevent spam), and for maintaining a "consensus set" (a list of addresses who are allowed to post transactions).

To allow for fine grained metering, the SDK provides two separate
units in which to charge transaction fees ("gas" and "diesel").
The SDK also allows (and expects) the STF to process any proofs that are posted onto the DA layer to
allow honest provers to be rewarded for their work, and to allow
adaptive gas pricing depending on prover throughput.

## Required Methods

### Begin Slot

* **Usage:**
  * Called exactly once for each slot (DA layer block), prior to processing any of the batches included in that slot.
  
### Parse Batch

* **Usage:**
  * SHOULD perform a zero-copy deserialization of a batch into a `header` and a list of `transaction`s. This method may perform
  additional sanity checks, but is assumed to be computationally
  inexpensive. Expensive checks (such as signatures) SHOULD wait for the
  `begin_batch` or `deliver_tx` calls.
* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | msg           | bytes  | The raw contents of a DA layer tx        |
 | sender        | bytes  | The sender of the DA layer TX, as bytes  |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok       | BATCH  | A deserialized batch                     |
 | Err       | optional CONSENSUS_UPDATE  | An update to the set of sequencers, potentially slashing the batch's sponsor|

* Note: This response is a `Result` type - only one of Ok or Err will be populated

### Parse Proof

* **Usage:**
  * SHOULD perform a zero-copy deserialization of a blob of bytes into a `proof`. This method may perform
  additional sanity checks, but is assumed to be computationally
  inexpensive. Expensive checks (such as signatures) SHOULD wait for the
  `deliverproof` calls
* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | msg           | bytes  | The raw contents of a DA layer tx        |
 | sender        | bytes  | The sender of the DA layer TX, as bytes  |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok       | PROOF  | A deserialized proof                     |
 | Err       | optional CONSENSUS_UPDATE  | An update to the set of sequencers, potentially slashing the proof's sponsor|

* Note: This response is a `Result` type - only one of Ok or Err will be populated

### Begin Batch

* **Usage:**
  * This method has two purposes: to allow the rollup to perofrm and  needed initialiation before
  processing the block, and to process an optional "misbehavior proof" to allow short-circuiting
  in case the block is invalid. (An example misbehavior proof would be a merkle-proof to a transaction
  with an invalid signature). In case of misbehavior, this method should slash the block's sender.
  TODO: decide whether to add events to the response

* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | batch   | BATCH  | The batch to be processed |
 | sender   | bytes  | The sender of the DA layer TX, as bytes |
 | misbehavior | optional MISBEHAVIOR_PROOF |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok       | _  | no response |
 | Err       | optional CONSENSUS_UPDATE  | An update to the set of sequencers, potentially slashing the batch's sender |

* Note: This response is a `Result` type - only one of Ok or Err will be populated

### Deliver TX

* **Usage:**
  * The core of the state transition function - called once for each rollup transaction. MUST NOT commit any changes
  to the rollup's state. Changes may only be persisted by the `end_batch`, `end_slot`, and `deliver_proof` method calls.
  This allows us to maintain the invariant that transactions contained in invalid batches are never committed,
  even in the face of a malicious prover who fails to supply a misbehavior proof during `begin_block`.

* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | batch   | BATCH  | The batch to be processed |
 | sender   | bytes  | The sender of the DA layer TX, as bytes |
 | misbehavior | optional MISBEHAVIOR_PROOF |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok       | DELIVER_TX_RESPONSE  | See |
 | Err       | optional CONSENSUS_UPDATE  | An update to the set of sequencers, potentially slashing the batch's sender |

* Note: This response is a `Result` type - only one of Ok or Err will be populated

### End Batch

* **Usage:**
  * Called at the end of each rollup batch, after all transactions in the batch have been delivered
  to the rollup's state. The rollup may use this call to persist any changes made during the course
  of the batch. The Sovereign SDK guarantees that no transaction in the batch will be reverted after this call is made
  unless the underlying DA layer experiences a fork.

* **Arguments**

None

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | response | END_BATCH_RESPONSE  | A list of updates to the rollups consensus set |

### End Slot

* **Usage:**
  * Called at the end of each slot, after all batches and proofs have been delivered.
  The rollup may use this call to persist any changes made during the course
  of the slot. The Sovereign SDK guarantees that no batch in the slot will be reverted after this call is made
  unless the underlying DA layer experiences a fork.

* **Arguments**

None

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | state root       | STATE_COMMITMENT  | A commitment to the rollup's state after this batch has been applied |

### Deliver Proof

* **Usage:**
  * Called between `begin_slot` and `end_slot` to process the completed proving of some prior transactions. May be
  invoked zero or more times per slot. May not be invoked between a `begin_batch` call and its corresponding `end_batch`.
  Rollups SHOULD use this call to compensate provers and adjust gas prices.

* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | sender | bytes  | The address which posted this proof on the L1 |
 | proof | PROOF | The deserialized proof |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok   | DELIVER_PROOF_RESPONSE  | A success response indicates how much gas/diesel was proved |
 | Err   | optional CONSENSUS_UPDATE  | An optional change to the consensus set to slash the sender |

* Note: This response is a `Result` type - only one of Ok or Err will be populated

## Structs

### Batch

| Name          | Type   | Description                              |
|---------------|--------|------------------------------------------|
| header        | HEADER | A batch header as defined by the STF|
| transactions  | repeated TRANSACTION | All of the transactions in this batch |

### Consensus Update

| Name          | Type   | Description                              |
|---------------|--------|------------------------------------------|
| address | bytes | A serialized address from the underlying DA layer |
| power  | u64 | The latest staked balance of this address |

### Transaction

A transaction on the STF. Likely contains a signature and some additional data.
Transactions are completely opaque to the Sovereign SDK

### Header

A batch header contains any information posted on-chain alongside the STF transactions.
The header format MAY be completely opaque to the Sovereign SDK. The header
MAY be zero-sized type - in which case it will be optimized away by the Rust compiler.
Note that a batch header is *not* the same as a header of the *logical* chain maintained by the SDK
or a header of the DA layer.

### Proof

A zero-knowledge proof of the validity of some state transition which extends the
previous best chain. For more details, see the [zkVM spec](./zkvm.md)

### Misbehavior Proof

An STF-defined type pointing out some malfeasance by the sequencer. For example, this type could simply contain an index
into the array of transactions, pointing out one which had an invalid signature.

### End Batch Response

| Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | sequencer_updates | repeated CONSENSUS_UPDATE  | A list of changes to the sequencer set|
 | prover_updates | repeated CONSENSUS_UPDATE  | A list of changes to the prover set|

### Deliver Tx Response

| Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | code | u32  | A response code. Following ABCI, 0 indicates success. |
 | data | bytes | Arbitrary data  |
 | gas_wanted | u64 | The amount of computational gas reserved for the transaction  |
 | gas_used | u64 | The amount of computational gas consumed by the transaction  |
 | diesel_wanted | u64 | The amount of computational diesel reserved for the transaction  |
 | diesel_used | u64 | The amount of computational gas consumed by the transaction  |
 | events | EVENT | A set of key-value pairs for indexing |

* Note: Deliver Tx responses should be committed to in the proof (just like the receipts root is included in the Ethereum
block header)

* Note: we introduce two "gas" types to allow for multi-dimensional fee markets. We use the term "diesel" to indicate that
the two kinds of gas are similar in many ways but are *not* interchangeable.

### Event

| Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | key | bytes  | The key, used to index this event |
 | value | bytes  | The value, to be returned when the index is queried |

### Deliver Proof Response

| Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | gas_proved | u64  | The amount of gas consumed by transactions covered by this proof |
 | diesel_proved | u64  | The amount of gas consumed by transactions covered by this proof |

TODO: consider adding pre and post state roots

## Optional Methods

TODO: consider adding functionality for 2-way bridging
