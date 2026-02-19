#!/usr/bin/env python3
"""Translate Hive geth-style genesis into sov-demo-rollup module genesis files.

This adapter intentionally starts small:
- EOA-focused mapping (alloc addresses become EVM accounts with empty code)
- balances are funded via bank.json (not via EVM account state)
- /chain.rlp and /blocks/*.rlp are intentionally ignored in this phase
"""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

EMPTY_CODE_HASH = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
DEFAULT_CHAIN_ID = 7


def parse_int(value, field_name: str) -> int:
    if value is None:
        raise ValueError(f"Missing integer field: {field_name}")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        v = value.strip()
        if v.startswith(("0x", "0X")):
            return int(v, 16)
        return int(v, 10)
    raise ValueError(f"Invalid integer field {field_name}: {value!r}")


def parse_maybe_int(value, fallback: int) -> int:
    if value is None:
        return fallback
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        v = value.strip()
        if v.startswith(("0x", "0X")):
            return int(v, 16)
        return int(v, 10)
    return fallback


def normalize_hex_address(address: str) -> str:
    if not isinstance(address, str):
        raise ValueError(f"Address must be string, got {type(address).__name__}")
    if not address.startswith(("0x", "0X")):
        raise ValueError(f"Expected 0x-prefixed address, got {address}")
    body = address[2:]
    if len(body) != 40:
        raise ValueError(f"Expected 20-byte address, got {address}")
    return "0x" + body.lower()


def balance_key(address: str) -> str:
    if isinstance(address, str) and address.startswith(("0x", "0X")):
        return address.lower()
    return address


def main() -> int:
    if len(sys.argv) != 4:
        print(
            "Usage: genesis_adapter.py <geth_genesis.json> <template_genesis_dir> <output_dir>",
            file=sys.stderr,
        )
        return 1

    input_genesis = Path(sys.argv[1])
    template_dir = Path(sys.argv[2])
    output_dir = Path(sys.argv[3])

    if not input_genesis.exists():
        print(f"Missing genesis file: {input_genesis}", file=sys.stderr)
        return 1
    if not template_dir.exists():
        print(f"Missing template genesis directory: {template_dir}", file=sys.stderr)
        return 1

    if output_dir.exists():
        shutil.rmtree(output_dir)
    shutil.copytree(template_dir, output_dir)

    with input_genesis.open("r", encoding="utf-8") as f:
        geth_genesis = json.load(f)

    evm_path = output_dir / "evm.json"
    bank_path = output_dir / "bank.json"

    with evm_path.open("r", encoding="utf-8") as f:
        evm_genesis = json.load(f)
    with bank_path.open("r", encoding="utf-8") as f:
        bank_genesis = json.load(f)

    chain_id = parse_maybe_int(geth_genesis.get("config", {}).get("chainId"), DEFAULT_CHAIN_ID)

    evm_genesis["genesis_timestamp"] = parse_maybe_int(
        geth_genesis.get("timestamp"),
        parse_maybe_int(evm_genesis.get("genesis_timestamp"), 0),
    )
    evm_genesis["initial_base_fee"] = parse_maybe_int(
        geth_genesis.get("baseFeePerGas"),
        parse_maybe_int(evm_genesis.get("initial_base_fee"), 0),
    )

    chain_spec = evm_genesis.setdefault("chain_spec", {})
    existing_block_gas_limit = parse_maybe_int(chain_spec.get("block_gas_limit"), 30_000_000)
    chain_spec["block_gas_limit"] = parse_maybe_int(
        geth_genesis.get("gasLimit"),
        existing_block_gas_limit,
    )

    tx_gas_limit = chain_spec.get("tx_gas_limit")
    tx_gas_limit = parse_maybe_int(tx_gas_limit, chain_spec["block_gas_limit"])
    if tx_gas_limit > chain_spec["block_gas_limit"]:
        tx_gas_limit = chain_spec["block_gas_limit"]
    chain_spec["tx_gas_limit"] = tx_gas_limit

    coinbase = geth_genesis.get("coinbase")
    if isinstance(coinbase, str) and coinbase.startswith(("0x", "0X")):
        try:
            chain_spec["coinbase"] = normalize_hex_address(coinbase)
        except ValueError:
            # Keep template value if geth coinbase is malformed.
            pass

    alloc = geth_genesis.get("alloc", {})
    if not isinstance(alloc, dict):
        raise ValueError("geth genesis alloc must be an object")

    evm_accounts = []
    alloc_balances = []
    skipped_non_eoa = 0

    for raw_address, alloc_entry in alloc.items():
        address = normalize_hex_address(raw_address)
        entry = alloc_entry if isinstance(alloc_entry, dict) else {}

        code = entry.get("code")
        if isinstance(code, str) and code not in ("", "0x", "0X"):
            skipped_non_eoa += 1

        if entry.get("storage"):
            skipped_non_eoa += 1

        evm_accounts.append(
            {
                "address": address,
                "code_hash": EMPTY_CODE_HASH,
                "code": "0x",
            }
        )

        balance_raw = entry.get("balance", "0x0")
        balance = parse_int(balance_raw, f"alloc[{raw_address}].balance")
        if balance < 0:
            raise ValueError(f"Negative alloc balance for {raw_address}")
        if balance > 0:
            alloc_balances.append((address, balance))

    evm_genesis["accounts"] = evm_accounts

    existing_balances = bank_genesis["gas_token_config"]["address_and_balances"]
    merged = {}
    order = []

    def add_balance(addr: str, amount: int) -> None:
        key = balance_key(addr)
        if key not in merged:
            merged[key] = [addr, 0]
            order.append(key)
        merged[key][1] += amount

    for addr, amount in existing_balances:
        add_balance(addr, parse_int(amount, f"bank.address_and_balances[{addr}]"))

    for addr, amount in alloc_balances:
        add_balance(addr, amount)

    bank_genesis["gas_token_config"]["address_and_balances"] = [
        [merged[key][0], str(merged[key][1])] for key in order
    ]

    with evm_path.open("w", encoding="utf-8") as f:
        json.dump(evm_genesis, f, indent=2)
        f.write("\n")

    with bank_path.open("w", encoding="utf-8") as f:
        json.dump(bank_genesis, f, indent=2)
        f.write("\n")

    (output_dir / "chain_id.txt").write_text(f"{chain_id}\n", encoding="utf-8")

    if evm_accounts:
        (output_dir / "smoke_address.txt").write_text(
            f"{evm_accounts[0]['address']}\n", encoding="utf-8"
        )

    print(
        f"Generated Hive genesis at {output_dir} (chain_id={chain_id}, alloc_accounts={len(evm_accounts)}, skipped_non_eoa={skipped_non_eoa})",
        file=sys.stderr,
    )

    if skipped_non_eoa > 0:
        print(
            "Note: non-EOA alloc entries (code/storage) were intentionally flattened to EOAs in this first pass.",
            file=sys.stderr,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
