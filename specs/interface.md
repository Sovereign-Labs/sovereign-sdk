# State transition Function

## Methods

### Begin Slot

* **Usage:**
  * Called exactly once for each DA-layer block, prior to processing any of the transactions included in that block.
  
### Parse Block

* **Usage:**
  * SHOULD perform a zero-copy deserialization of a block into a `header` and a list of `transaction`s. This method may perform
  additional sanity checks, but is assumed to be computationally
  inexpensive. Expensive checks (such as signatures) SHOULD wait for the
  `begin_block` or `deliver_tx` calls
* **Arguments**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | msg           | bytes  | The raw contents of a DA layer tx        |
 | sender        | bytes  | The sender of the DA layer TX, as bytes  |

* **Response**

 | Name          | Type   | Description                              |
 |---------------|--------|------------------------------------------|
 | Ok       | BLOCK  | A deserialized block                     |
 | Err       | optional CONSENSUS_UPDATE  | An update to the set of sequencers, potentially slashing the block's sponsor|

* Note: This response is a `Result` type - only one of Ok or Err will be populated

## Structs

### Block

| Name          | Type   | Description                              |
|---------------|--------|------------------------------------------|
| header        | HEADER | The raw contents of a DA layer tx        |
| transactions  | repeated TRANSACTION | All of the transactions in this block |

### Transaction

A transaction on the rollup. Likely contains a signature and some additional data.
Transactions are completely opaque to the Sovereign SDK

### Header

A header contains any information posted on-chain alongside the rollup transactions.
The rollup header format may be completely opaque to the Sovereign SDK. The header
may be totally empty.
