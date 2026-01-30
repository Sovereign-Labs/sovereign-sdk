# Sovereign Web3 Python

A Python wrapper for the Sovereign SDK's web3 functionality, enabling transaction creation, signing, and submission to Sovereign rollups.

## Features

- **Transaction Creation**: Build unsigned transactions with runtime calls and transaction details
- **Schema Handling**: Fetch and use rollup schemas from URLs or JSON for proper serialization
- **Transaction Signing**: Generate signing payloads and create signed transactions
- **Serialization**: Convert transactions to bytes for network submission

## Installation

```bash
pip install sovereign_web3
```

## Usage

```python
from sovereign_web3 import Serializer, UnsignedTransaction, TxDetails

# Fetch schema from rollup
serializer = Serializer.from_url("http://localhost:12346/rollup/schema")

# Create transaction
call = {"bank": {"create_token": {"token_name": "MyToken", "initial_balance": "1000"}}}
details = TxDetails(chain_id=4321)
unsigned_tx = UnsignedTransaction(runtime_call=call, details=details)

# Get bytes for signing
tx_bytes = unsigned_tx.bytes_for_signing(serializer)

# Sign and create final transaction
signed_tx = unsigned_tx.to_signed(pub_key=public_key, signature=signature)
serialized = serializer.serialize_tx(signed_tx)
```

## Classes

- `Serializer`: Schema-based transaction serialization
- `UnsignedTransaction`: Unsigned transaction with runtime calls
- `TxDetails`: Transaction metadata (chain ID, fees, gas)
- `UniquenessData`: Transaction uniqueness (nonce or generation)
