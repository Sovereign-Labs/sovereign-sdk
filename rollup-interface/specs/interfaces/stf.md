# State transition Function

## Overview

The state transition function is the "business logic" of a rollup.
It is defined in a particular zkVM (since implementation details like hash and proof formats functions need to change). However,
an implementation of an STF may be reused across multiple DA layers.

The Sovereign SDK guarantees that all relevant data blobs will be delivered to the STF for processing
exactly once, in the order that they appear on the DA layer. The STF is responsible for implementing its own metering
and billing (to prevent spam).

The SDK also allows (and expects) the STF to process any proofs that are posted onto the DA layer to
allow honest provers to be rewarded for their work, and to allow
adaptive gas pricing depending on prover throughput.

## Required Methods

### Init Chain

- **Usage:**

  - Called exactly once at the rollup's genesis, prior to processing batches.
    This method is used to perform one-time initialization, such as minting the rollup's native token.

- **Arguments**

| Name   | Type          | Description                             |
|--------|---------------|-----------------------------------------|
| params | INITIAL_STATE | The initial state to set for the rollup |

### Begin Slot

- **Usage:**

  - Called exactly once for each slot (DA layer block), prior to processing any of the batches included in that slot.
    This method is invoked whether or not the slot contains any data relevant to the rollup.

- **Arguments**

| Name    | Type    | Description                                                                                                                                                                                                                     |
|---------|---------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| witness | WITNESS | The witness to be used to process this slot. In prover mode, the witness argument is an empty struct which is populated with "hints" for the ZKVM during execution. In ZK mode, the argument is the pre-populated set of hints. |

### Apply Blob

- **Usage:**

  - This method is called once for each blob sent by the DA layer. It should attempt
    to interpret each as a message for the rollup and apply any resulting state
    transitions.
    It accepts an optional "misbehavior proof" to allow short-circuiting
    in case the block is invalid. (An example misbehavior proof would be a merkle-proof to a transaction
    with an invalid signature).

- **Arguments**

| Name        | Type                       | Description                                                                                                  |
| ----------- | -------------------------- | ------------------------------------------------------------------------------------------------------------ |
| blob        | BLOB_TRANSACTION           | A struct containing the blob's data and the address of the sender                                            |
| misbehavior | optional MISBEHAVIOR_PROOF | Gives the rollup a hint that misbehavior has occurred, allowing the state-transition to be "short-circuited" |

- **Response**

| Name    | Type          | Description                                                                                                                                                |
| ------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| receipt | BATCH_RECEIPT | A receipt indicating whether the blob was applied succesfully. Contains an array of `TX_RECEIPT`s, indicating the result of each transaction from the blob |

### End Slot

- **Usage:**

  - Called at the end of each slot, after all batches and proofs have been delivered.
    The rollup may use this call to persist any changes made during the course
    of the slot. The Sovereign SDK guarantees that no batch in the slot will be reverted after this call is made
    unless the underlying DA layer experiences a reorg.

- **Arguments**

None

- **Response**

| Name       | Type       | Description                                                                                                                                                 |
| ---------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| state root | STATE_root | A commitment to the rollup's state after this batch has been applied                                                                                        |
| witness    | WITNESS    | In prover mode, return the accumulated "hints" which the ZKVM will need to verify the state transition efficiently. In ZK mode, this return value is unused |

## Structs

### Blob

| Name   | Type  | Description                                                                              |
| ------ | ----- | ---------------------------------------------------------------------------------------- |
| sender | bytes | The address of the entity which sent this blob of data to the DA layer, as a byte string |
| data   | bytes | The content of this blob as an iterable collection of bytes                              |

### InitialState

A rollup-defined type which is opaque to the rest of the SDK. Specifies the genesis
state of a particular instance of the state transition function.

### Misbehavior Proof

An STF-defined type pointing out some malfeasance by the sequencer. For example, this type could simply contain an index
into the array of transactions, pointing out one which had an invalid signature.

### Event

| Name  | Type  | Description                                        |
| ----- | ----- | -------------------------------------------------- |
| key   | bytes | The key used to index this event                   |
| value | bytes | The value to be returned when the index is queried |

### BatchReceipt

| Name           | Type                        | Description                                                                  |
| -------------- | --------------------------- | ---------------------------------------------------------------------------- |
| batch_hash     | bytes                       | The canonical hash of this batch                                             |
| tx_receipts    | repeated TransactionReceipt | A receipt for each transaction included in this batch                        |
| custom_receipt | CUSTOM_BATCH_RECEIPT        | extra data to be stored as part of this batch's receipt, custom for each STF |

### TransactionReceipt

| Name           | Type              | Description                                                                            |
| -------------- | ----------------- | -------------------------------------------------------------------------------------- |
| tx_hash        | bytes             | The canonical hash of this transaction                                                 |
| body_to_save   | optional bytes    | The canonical representation of this transaction to be stored by full nodes if present |
| events         | repeated Event    | The value, to be returned when the index is queried                                    |
| custom_receipt | CUSTOM_TX_RECEIPT | extra data to be stored as part of this transaction's receipt, custom for each STF     |

### Witness

A custom type for each state transition function containing the hints that are passed to the ZKVM.

## Optional Methods

TODO: consider adding functionality for 2-way bridging
