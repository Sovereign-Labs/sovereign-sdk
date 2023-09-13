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
| ------ | ------------- | --------------------------------------- |
| params | INITIAL_STATE | The initial state to set for the rollup |

### Apply Slot

- **Usage:**

  - Called exactly once for each slot (DA layer block) to allow the rollup to process the data from that slot.
    This method is invoked whether or not the slot contains any data relevant to the rollup.

- **Arguments**

| Name               | Type               | Description                                                                                                                                                                                                                                                                                                                        |
| ------------------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| witness            | WITNESS            | The witness to be used to process this slot. In prover mode, the witness argument is an empty struct which is populated with "hints" for the ZKVM during execution. In ZK mode, the argument is the pre-populated set of hints.                                                                                                    |
| slot_header        | BLOCK_HEADER       | The header of the block on the DA layer                                                                                                                                                                                                                                                                                            |
| validity_condition | VALIDITY_CONDITION | Data for any extra checks which must be made by light clients before accepting the state transition from this slot. For example, if the DA layer uses a verkle tree which is too expensive to open in-circuit, this might contain a merkle root of the observed slot data - which light clients would need to check "out-of-band". |
| blobs              | BLOB_TRANSACTIONS  | An iterator over the blobs included in this slot                                                                                                                                                                                                                                                                                   |

- **Response**

| Name   | Type        | Description                                                                                  |
| ------ | ----------- | -------------------------------------------------------------------------------------------- |
| result | SLOT_RESULT | The new state root, receipts, and witness resulting from the application of this slot's data |

### Get Current State Root

- **Usage:**

  - Gets the current state root from the frollup

- **Arguments**

  - None

- **Response**

| Name       | Type                | Description                   |
| ---------- | ------------------- | ----------------------------- |
| state_root | optional STATE_ROOT | The current rollup state root |

## Structs

### Blob

| Name   | Type  | Description                                                                              |
| ------ | ----- | ---------------------------------------------------------------------------------------- |
| sender | bytes | The address of the entity which sent this blob of data to the DA layer, as a byte string |
| data   | bytes | The content of this blob as an iterable collection of bytes                              |

### InitialState

A rollup-defined type which is opaque to the rest of the SDK. Specifies the genesis
state of a particular instance of the state transition function.

### Event

| Name  | Type  | Description                                        |
| ----- | ----- | -------------------------------------------------- |
| key   | bytes | The key used to index this event                   |
| value | bytes | The value to be returned when the index is queried |

### SlotResult

| Name           | Type           | Description                                                                                                                                                             |
| -------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| state_root     | STATE_ROOT     | The current state root of the rollup, calculated after applying the state transition function to the data from this slot.                                               |
| batch_receipts | BATCH_RECEIPTs | A list of receipt indicating whether each blob was applied succesfully. Each receipt an array of `TX_RECEIPT`s, indicating the result of each transaction from the blob |
| witness        | WITNESS        | The witness generated from this execution of the state transition function                                                                                              |

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
