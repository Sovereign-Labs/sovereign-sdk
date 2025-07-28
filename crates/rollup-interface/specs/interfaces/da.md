# Data Availability Layer

## Required Functionality

The DA layer is responsible for several tasks:

1. Ensuring the availability of rollup data
1. Rate limiting and prioritizing the inbound batches that a rollup is forced to process
1. Providing tamper-proof attestations to the sender of batches (as long as rate-limiting is enforced, this is optional)
1. Providing a backstop for censorship resistance
1. Providing a total ordering of batches within and between rollups on the chain

Let's go through each of these tasks in detail:

### Data Availability

Rollups are designed to allow arbitrary state machines to inherit security guarantees
of an underlying L1 blockchain. In order for these guarantees to hold, the rollup's data must be "available".
Otherwise, an adversary can freeze the rollup by publishing a new state root that incorporates a valid
but secret state update. Since no one else knows the new state of the system, honest participants will
be unable to post new blocks.

In the Sovereign SDK, we assume that data published on an underlying L1 is available, and simply check in-proof
that the data in question really is included in the L1's history. It is the responsibility of the rollup developer
to choose a chain that provides suitable data availability guarantees for their application.

### Rate Limiting and Priority

No chain has infinite capacity. So, in order to ensure robust operation, chains need to ensure that transactions
can be prioritized. Since transactions on the rollup create useful economic activity, we assume that, over time,
honest sequencers, including "real" transactions, should be able to generate revenue. They can use this revenue
to bid for blockspace on the L1. By contrast, dishonest sequencers sending only "spam" transactions do not generate
revenue. For DOS resistance, it's vital that these dishonest sequencers not be able to "crowd out" honest sequencers
permanently. So, sending transactions on the L1 must be costly. In addition, the fee paid on the L1 should be
proportional to the demand, so that the cost of crowding out honest rollup transactions rises with the value
of those transactions.

### Sender Attestation

One underappreciated strength of layer 1 blockchains is their ability to prune
out invalid transactions before they get included in blocks. Since most academic work on blockchains abstracts
the peer-to-peer layer as a simple gossip network, it has not (to our knowledge) been pointed out that L1s
"hyperscale" in their ability to weed out invalid transactions. In other words, the ability of a typical L1
to withstand a DOS attack _based on invalid transactions_ scales linearly with the number of full nodes.

One simple DOS attack on a blockchain is to submit a large number of plausible-looking transactions into the mempool,
all of which have invalid signatures. Since the signatures are invalid, nobody can be charged on-chain for the spam.
But, since the transactions look plausible, full nodes have to do the work of checking the signatures. Since it's
much cheaper to pick some random bytes that look like a signature than it is to check the signature's validity,
a resource constrained attacker can launch a fairly effective spam attack with this method.

But, L1s aren't vulnerable. Why not? Because full nodes refuse to gossip invalid transactions, disconnect
from any nodes that do, and only open new connections at a limited rate. This prevents the attacker from
either overwhelming individual peers or - even worse - getting his spam transactions included in the final
ledger.

Because Sovereign SDK chains are designed to operate over a "lazy" ledger which contains invalid transactions,
they don't inherit this property by default. To compensate, the Sovereign
SDK uses a simple trick: we force sequencers to register
on the L2 chain by bonding some (L2) tokens _and_ claiming ownerships of an L1 address. Using this
trick, we can offload the sequencer signature checks to the L1. At the rollup level, we only process batches
that have been sent by bonded sequencers (who we can slash to disincentivize spam), and we use the fact
that the L1 is enforcing signature checks to offload work to the L1 consensus network. So, rather than
making an expensive signature check for each batch in zk, we use a much cheaper lookup to check if the
user is a registered sequencer.

### Censorship Resistance

Censorship resistance is the whole point. If your rollup doesn't have it, it's not an L2, just a sparkling database.
But, a rollup can't provide censorship resistance on its own - after all, the underlying L1 could always censor
bundles containing unpopular transactions. So, the L1 needs to be censorship resistant.

### Total Ordering

We allow Sovereign SDK chains to specify any state model of their choosing. So, the underlying DA layer must
provide a total ordering over rollup batches. For purposes of bridging, it also needs
to provide an ordering _across_ batches on different rollups. (This is only an issue if the underlying
chain uses a DAG model). This requirement may be relaxed in the future.

## Optional Functionality

TODO: 2-way trust-minimized bridge with rollup

# Required Interfaces

## DaSpec

This interface defines the types shared between the `DaService` and `DaVerifier` traits. It has no associated
functions.

### Type: `BlobTransaction`

| Name     | Type      | Description                                                            |
| -------- | --------- | ---------------------------------------------------------------------- |
| `sender` | `Address` | The address which sent this transaction                                |
| `data`   | `bytes`   | Data intended for rollup. Can be a proof, a transaction list, and etc. |

### Type: `InclusionMultiproof`

A proof showing that each item in an associated vector is included in some state commitment. For example,
this could be a list of merkle siblings.

### Type: `CompletenessProof`

A proof showing that an associated vector does not omit any "relevant" transactions. For example, this could be a
merkle proof of the items immediately preceding and following a particular Celestia namespace. This type may be
the unit struct if no completeness proof is required.

### Type: `Blockheader`

Must include a `prev_hash` field. Must provide a function to compute its canonical hash.

| Name        | Type       | Description                         |
| ----------- | ---------- | ----------------------------------- |
| `prev_hash` | `SlotHash` | the hash of the previous (L1) block |

### Type: `SlotHash`

The hash of a DA layer block. May be any type, but must provide access to a (canonical) byte string
uniquely representing this hash.

| Name    | Type    | Description               |
| ------- | ------- | ------------------------- |
| `inner` | `bytes` | The raw bytes of the hash |

**Code**

Expressed in Rust, the DA Spec interface is a `trait`. You can find the trait implementation [here](../../src/state_machine/da.rs).

## DaVerifier

The DaVerifier trait is part the rollup's state machine - meaning that it has to be proven in zk. Its job is to ensure that
the DA layer's consensus translates into rollup state.

### Method:`verify_relevant_tx_list`

- **Usage:**

  - An adaptation of the `get_relevant_txs` method designed for use by verifiers. This method
    returns a `Result` containing the unit type on success, and an error message on failure. An invocation
    succeeds if and only if the provided set of `txs` was included on the L1 (as demonstrated by `inclusion_proof`),
    and is complete (as demonstrated by `completeness_proof`). The list of blob transactions verified by this trait will be
    processed by rollups in order - which means that the verification must be careful to enforce a deterministic ordering
    to prevent consensus failures on the rollup. Except for the "Error" response, all of the types required by this function are
    defined in the `DaSpec` trait.

- **Arguments:**

| Name                 | Type                        | Description                                                                |
| -------------------- | --------------------------- | -------------------------------------------------------------------------- |
| `block_header`       | `Blockheader`               | The header of the DA layer block including the relevant transactions       |
| `txs`                | `iterable<BlobTransaction>` | A list of L1 transactions ("data blobs"), with their senders               |
| `inclusion_proof`    | `InclusionMultiproof`       | A witness showing that each transaction was included in the DA layer block |
| `completeness_proof` | `CompletenessProof`         | A witness showing that the returned list of transactions is complete       |

- **Response:**

| Name  | Type    | Description      |
| ----- | ------- | ---------------- |
| `Ok`  | `_`     | No response      |
| `Err` | `Error` | An error message |

- Note: This response is a `Result` type - only one of Ok or Err will be populated

**Code**

Expressed in Rust, the DA Verifier interface is a `trait`. You can find the trait implementation [here](../../src/state_machine/da.rs).

## DaService

The DA Service interface allows the Sovereign SDK to interact with a DA layer. It's responsible for communicating with the DA layer's
peer network (likely via JSON-RPC requests to a local light node), and for converting information about the DA layer into a form
which can be efficiently verified in the SNARK. The DA service exists outside of the rollup's state machine, so it will not be proven
in-circuit. For this reason, implementers are encouraged to prioritize readability and maintainability over efficiency.

### Method:`extract_relevant_blobs`

- **Usage:**

  - The core of the DA service. Fetches all "relevant" transactions from a given DA layer block.
    The exact criteria that make transactions "relevant" are up to the implementer, but must be
    defined without reference to the current state of the rollup. For example, a rollup on Celestia
    might define "relevant" to mean, "occurring in namespace 'foo'".

- **Arguments:**

| Name    | Type            | Description                                       |
| ------- | --------------- | ------------------------------------------------- |
| `block` | `FilteredBlock` | The relevant subset of data from a DA layer block |

- **Response:**

| Name  | Type                        | Description                                                  |
| ----- | --------------------------- | ------------------------------------------------------------ |
| `txs` | `iterable<BlobTransaction>` | A list of L1 transactions ("data blobs"), with their senders |

### Method:`extract_relevant_blobs_with_proof`

- **Usage:**

  - An adaptation of the `extract_relevant_blobs` method designed for use by provers. This method
    returns the same list of transactions that would be returned by `extract_relevant_blobs`, in addition
    to a witness proving the inclusion of these transactions in the DA layer block, and a witness
    showing the completeness of the provided list. The output of this function is intended to be
    passed to the `DaVerifier`.

- **Arguments:**

| Name    | Type            | Description                                       |
| ------- | --------------- | ------------------------------------------------- |
| `block` | `FilteredBlock` | The relevant subset of data from a DA layer block |

- **Response:**

| Name                 | Type                        | Description                                                                |
| -------------------- | --------------------------- | -------------------------------------------------------------------------- |
| `txs`                | `iterable<BlobTransaction>` | A list of L1 transactions ("data blobs"), with their senders               |
| `inclusion_proof`    | `InclusionMultiproof`       | A witness showing that each transaction was included in the DA layer block |
| `completeness_proof` | `CompletenessProof`         | A witness showing that the returned list of transactions is complete       |

### Method:`get_finalized_at`

- **Usage:**

  - Fetch the (relevant portion of the) finalized block at the given height. If `height` has not been finalized, block
    until it is.

- **Arguments:**

| Name     | Type  | Description                       |
| -------- | ----- | --------------------------------- |
| `height` | `u64` | The height of the block to return |

- **Response:**

| Name    | Type            | Description                                       |
| ------- | --------------- | ------------------------------------------------- |
| `block` | `FilteredBlock` | The relevant subset of data from a DA layer block |

### Method:`get_block_at`

- **Usage:**

  - Fetch the (relevant portion of the) block at the given height. If `height` has not yet been reached, block
    until it is. A response may be returned before it is "finalized" by the underlying consensus.

- **Arguments:**

| Name     | Type  | Description                       |
| -------- | ----- | --------------------------------- |
| `height` | `u64` | The height of the block to return |

- **Response:**

| Name    | Type            | Description                                       |
| ------- | --------------- | ------------------------------------------------- |
| `block` | `FilteredBlock` | The relevant subset of data from a DA layer block |

### Method:`send_transaction`

- **Usage:**

  - Post the provided blob of bytes onto the data availability layer

- **Arguments:**

| Name   | Type   | Description                      |
| ------ | ------ | -------------------------------- |
| `blob` | bytes` | The data to post on the DA layer |

- **Response:**

| Name  | Type    | Description      |
| ----- | ------- | ---------------- |
| `Ok`  | `_`     | No response      |
| `Err` | `Error` | An error message |

### Type: `RuntimeConfig`

A struct containing whatever runtime configuration is necessary to initialize this `DaService`. For example, this
struct could contain the IP address and port of the remote RPC node that this `DaService` should connect to.

### Type: `FilteredBlock`

The relevant subset of data from the DA layer block. This type must contain all data which will be processed by the rollup
and enough auxiliary data to allow its block hash to be recomputed by the `DaVerifier`.

**Code**

Expressed in Rust, the DA Verifier interface is a `trait`. You can find the trait implementation [here](../../src/node/services/da.rs).
