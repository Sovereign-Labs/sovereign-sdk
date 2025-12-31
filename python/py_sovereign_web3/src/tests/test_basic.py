import base64
import requests
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from sovereign_web3 import Serializer, UnsignedTransaction, TxDetails


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
    details = TxDetails(chain_id=4321)
    unsigned_tx = UnsignedTransaction(runtime_call=call, details=details)
    tx_bytes = unsigned_tx.bytes_for_signing(serializer)

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
