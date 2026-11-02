import base64
from pathlib import Path

import pytest
import requests
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from sovereign_web3 import Serializer, UnsignedTransaction, TxDetails, UniquenessData


def test_commonware_signing_payload():
    schema_path = (
        Path(__file__).resolve().parents[4]
        / "examples/demo-rollup/demo-rollup-schema.json"
    )
    serializer = Serializer(schema_path.read_text())
    fragment = serializer.chain_hash_fragment()
    unsigned_tx = UnsignedTransaction(
        runtime_call={"chain_state": "TerminateSetupMode"},
        details=TxDetails(
            chain_hash_fragment=fragment, max_fee=100, max_priority_fee_bips=2
        ),
        uniqueness=UniquenessData.nonce(7),
    )
    # Borsh: runtime call, nonce, fees, gas limit, chain fragment, address override.
    body = (
        b"\x07\x00\x00"
        + (7).to_bytes(8, "little")
        + (2).to_bytes(8, "little")
        + (100).to_bytes(16, "little")
        + b"\x00"
        + fragment.to_bytes(8, "little")
        + b"\x00"
    )
    payload = b"\x00" + body + bytes(serializer.chain_hash())
    raw_signing_bytes = unsigned_tx.bytes_for_signing(serializer)
    assert raw_signing_bytes == payload
    assert serializer.serialize_signing_payload(unsigned_tx) == payload
    signing_bytes = unsigned_tx.commonware_bytes_for_signing(serializer)
    assert signing_bytes == b"\x00" + payload
    assert serializer.serialize_commonware_signing_payload(unsigned_tx) == signing_bytes

    private_key = Ed25519PrivateKey.from_private_bytes(bytes([0x5E]) * 32)
    public_key = private_key.public_key()
    raw_signature = private_key.sign(raw_signing_bytes)
    public_key.verify(raw_signature, payload)
    signature = private_key.sign(signing_bytes)
    public_key.verify(signature, b"\x00" + payload)
    with pytest.raises(InvalidSignature):
        public_key.verify(signature, payload)
    with pytest.raises(InvalidSignature):
        public_key.verify(signature, b"\x00\x00" + payload)

    pub_key = public_key.public_bytes(
        encoding=serialization.Encoding.Raw, format=serialization.PublicFormat.Raw
    )
    for tx_signature in (raw_signature, signature):
        signed_tx = unsigned_tx.to_signed(pub_key=pub_key, signature=tx_signature)
        assert serializer.serialize_tx(signed_tx) == b"\x00" + tx_signature + pub_key + body


def test_basic_transaction_submission():
    serializer = Serializer.from_url("http://0.0.0.0:12346/rollup/schema")
    call = {
        "bank": {
            "create_token": {
                "token_name": "smoke test",
                "initial_balance": "20000",
                "token_decimals": 12,
                "supply_cap": "100000000000",
                "mint_to_address": {
                    "Standard": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
                },
                "admins": [
                    {
                        "Standard": "sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf",
                    }
                ],
            }
        }
    }
    details = TxDetails(chain_hash_fragment=serializer.chain_hash_fragment())
    unsigned_tx = UnsignedTransaction(runtime_call=call, details=details)
    tx_bytes = unsigned_tx.commonware_bytes_for_signing(serializer)

    # sign tx
    private_key = Ed25519PrivateKey.generate()
    signature = private_key.sign(tx_bytes)
    pub_key = private_key.public_key().public_bytes(
        encoding=serialization.Encoding.Raw, format=serialization.PublicFormat.Raw
    )
    signed_tx = unsigned_tx.to_signed(pub_key=pub_key, signature=signature)

    # serialize and submit tx
    tx_bytes = serializer.serialize_tx(signed_tx)
    encoded_tx = base64.b64encode(tx_bytes).decode("utf-8")
    response = requests.post(
        "http://0.0.0.0:12346/sequencer/txs", json={"body": encoded_tx}
    )

    assert response.status_code == 200

    data = response.json()
    event = data["events"][0]

    assert event["key"] == "Bank/TokenCreated"
