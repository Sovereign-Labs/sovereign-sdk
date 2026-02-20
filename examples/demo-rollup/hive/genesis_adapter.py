#!/usr/bin/env python3
"""Translate Hive geth-style genesis into sov-demo-rollup module genesis files.

Current behavior:
- alloc balances are funded via bank.json (not via EVM account state)
- alloc code / nonce / storage are imported into evm.json accounts
- /chain.rlp is used only to infer time-based fork activation blocks
- /chain.rlp and /blocks/*.rlp historical bodies are not imported in this phase
"""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

EMPTY_CODE_HASH = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
DEFAULT_CHAIN_ID = 7
MASK_64 = (1 << 64) - 1

# Keccak-f[1600] round constants.
RC = [
    0x0000000000000001,
    0x0000000000008082,
    0x800000000000808A,
    0x8000000080008000,
    0x000000000000808B,
    0x0000000080000001,
    0x8000000080008081,
    0x8000000000008009,
    0x000000000000008A,
    0x0000000000000088,
    0x0000000080008009,
    0x000000008000000A,
    0x000000008000808B,
    0x800000000000008B,
    0x8000000000008089,
    0x8000000000008003,
    0x8000000000008002,
    0x8000000000000080,
    0x000000000000800A,
    0x800000008000000A,
    0x8000000080008081,
    0x8000000000008080,
    0x0000000080000001,
    0x8000000080008008,
]

# Rho offsets r[x][y].
RHO = [
    [0, 36, 3, 41, 18],
    [1, 44, 10, 45, 2],
    [62, 6, 43, 15, 61],
    [28, 55, 25, 21, 56],
    [27, 20, 39, 8, 14],
]


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


def parse_optional_int(value) -> int | None:
    if value is None:
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        v = value.strip()
        if v.startswith(("0x", "0X")):
            return int(v, 16)
        return int(v, 10)
    return None


def normalize_hex_address(address: str) -> str:
    if not isinstance(address, str):
        raise ValueError(f"Address must be string, got {type(address).__name__}")

    # Geth genesis alloc keys can be either "0x..." or plain 40-hex strings.
    body = address[2:] if address.startswith(("0x", "0X")) else address
    if len(body) != 40:
        raise ValueError(f"Expected 20-byte address (40 hex chars), got {address}")
    try:
        int(body, 16)
    except ValueError as exc:
        raise ValueError(f"Invalid hex address: {address}") from exc

    return "0x" + body.lower()


def rol64(value: int, shift: int) -> int:
    if shift == 0:
        return value & MASK_64
    return ((value << shift) | (value >> (64 - shift))) & MASK_64


def keccak_f1600(state: list[int]) -> None:
    for rc in RC:
        # Theta
        c = [0] * 5
        for x in range(5):
            c[x] = (
                state[x]
                ^ state[x + 5]
                ^ state[x + 10]
                ^ state[x + 15]
                ^ state[x + 20]
            )
        d = [0] * 5
        for x in range(5):
            d[x] = c[(x - 1) % 5] ^ rol64(c[(x + 1) % 5], 1)
        for x in range(5):
            for y in range(5):
                state[x + 5 * y] = (state[x + 5 * y] ^ d[x]) & MASK_64

        # Rho + Pi
        b = [0] * 25
        for x in range(5):
            for y in range(5):
                b[y + 5 * ((2 * x + 3 * y) % 5)] = rol64(
                    state[x + 5 * y], RHO[x][y]
                )

        # Chi
        for x in range(5):
            for y in range(5):
                idx = x + 5 * y
                state[idx] = (
                    b[idx]
                    ^ ((~b[((x + 1) % 5) + 5 * y]) & b[((x + 2) % 5) + 5 * y])
                ) & MASK_64

        # Iota
        state[0] = (state[0] ^ rc) & MASK_64


def keccak_256(data: bytes) -> bytes:
    # Keccak-256 sponge params: rate=1088 bits (136 bytes), capacity=512 bits.
    rate = 136
    state = [0] * 25

    offset = 0
    while offset + rate <= len(data):
        block = data[offset : offset + rate]
        for i in range(rate // 8):
            lane = int.from_bytes(block[8 * i : 8 * i + 8], "little")
            state[i] = (state[i] ^ lane) & MASK_64
        keccak_f1600(state)
        offset += rate

    # Keccak padding (not SHA3): domain suffix 0x01, final bit 0x80.
    tail = bytearray(data[offset:])
    tail.append(0x01)
    while len(tail) < rate:
        tail.append(0)
    tail[-1] |= 0x80

    for i in range(rate // 8):
        lane = int.from_bytes(tail[8 * i : 8 * i + 8], "little")
        state[i] = (state[i] ^ lane) & MASK_64
    keccak_f1600(state)

    out = bytearray()
    for i in range(rate // 8):
        out.extend(state[i].to_bytes(8, "little"))
        if len(out) >= 32:
            return bytes(out[:32])
    return bytes(out[:32])


def checksum_address(address: str) -> str:
    # EIP-55 checksum over lowercase hex (without 0x), using Keccak-256.
    normalized = normalize_hex_address(address)
    body = normalized[2:]
    hashed = keccak_256(body.encode("ascii")).hex()
    result = []
    for i, ch in enumerate(body):
        if ch.isdigit():
            result.append(ch)
        else:
            result.append(ch.upper() if int(hashed[i], 16) >= 8 else ch)
    return "0x" + "".join(result)


def balance_key(address: str) -> str:
    if isinstance(address, str) and address.startswith(("0x", "0X")):
        return address.lower()
    return address


def normalize_hex_data(value: str, field_name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"Expected hex string for {field_name}, got {type(value).__name__}")

    body = value[2:] if value.startswith(("0x", "0X")) else value
    if body == "":
        return "0x"
    if len(body) % 2 == 1:
        body = "0" + body
    try:
        int(body, 16)
    except ValueError as exc:
        raise ValueError(f"Invalid hex data for {field_name}: {value}") from exc
    return "0x" + body.lower()


def to_hex_u256(value: int, field_name: str) -> str:
    if value < 0:
        raise ValueError(f"Negative value for {field_name}: {value}")
    return hex(value)


def rlp_item_info(buf: bytes, pos: int) -> tuple[bool, int, int, int]:
    if pos >= len(buf):
        raise ValueError("RLP decode out of bounds")

    prefix = buf[pos]
    if prefix <= 0x7F:
        return (False, pos, 1, pos + 1)
    if prefix <= 0xB7:
        length = prefix - 0x80
        payload_start = pos + 1
        payload_end = payload_start + length
        if payload_end > len(buf):
            raise ValueError("RLP short string length out of bounds")
        return (False, payload_start, length, payload_end)
    if prefix <= 0xBF:
        len_of_len = prefix - 0xB7
        payload_len_start = pos + 1
        payload_start = payload_len_start + len_of_len
        if payload_start > len(buf):
            raise ValueError("RLP long string length prefix out of bounds")
        length = int.from_bytes(buf[payload_len_start:payload_start], "big")
        payload_end = payload_start + length
        if payload_end > len(buf):
            raise ValueError("RLP long string length out of bounds")
        return (False, payload_start, length, payload_end)
    if prefix <= 0xF7:
        length = prefix - 0xC0
        payload_start = pos + 1
        payload_end = payload_start + length
        if payload_end > len(buf):
            raise ValueError("RLP short list length out of bounds")
        return (True, payload_start, length, payload_end)

    len_of_len = prefix - 0xF7
    payload_len_start = pos + 1
    payload_start = payload_len_start + len_of_len
    if payload_start > len(buf):
        raise ValueError("RLP long list length prefix out of bounds")
    length = int.from_bytes(buf[payload_len_start:payload_start], "big")
    payload_end = payload_start + length
    if payload_end > len(buf):
        raise ValueError("RLP long list length out of bounds")
    return (True, payload_start, length, payload_end)


def rlp_list_items(
    buf: bytes, payload_start: int, payload_len: int
) -> list[tuple[bool, int, int, int]]:
    items: list[tuple[bool, int, int, int]] = []
    pos = payload_start
    end = payload_start + payload_len
    while pos < end:
        item = rlp_item_info(buf, pos)
        items.append(item)
        pos = item[3]
    if pos != end:
        raise ValueError("Malformed RLP list payload")
    return items


def rlp_string_to_int(buf: bytes, item: tuple[bool, int, int, int]) -> int:
    is_list, payload_start, payload_len, _ = item
    if is_list:
        raise ValueError("Expected RLP string, found list")
    if payload_len == 0:
        return 0
    return int.from_bytes(buf[payload_start : payload_start + payload_len], "big")


def load_chain_timestamps(chain_rlp_path: Path) -> list[tuple[int, int]]:
    data = chain_rlp_path.read_bytes()
    result: list[tuple[int, int]] = []
    pos = 0

    while pos < len(data):
        is_list, payload_start, payload_len, item_end = rlp_item_info(data, pos)
        if not is_list:
            raise ValueError("Top-level chain.rlp item must be a block list")

        block_items = rlp_list_items(data, payload_start, payload_len)
        if len(block_items) < 1:
            raise ValueError("Malformed block in chain.rlp")

        header_item = block_items[0]
        if not header_item[0]:
            raise ValueError("Malformed block header in chain.rlp")

        header_items = rlp_list_items(data, header_item[1], header_item[2])
        if len(header_items) < 12:
            raise ValueError("Block header has fewer fields than expected")

        block_number = rlp_string_to_int(data, header_items[8])
        timestamp = rlp_string_to_int(data, header_items[11])
        result.append((block_number, timestamp))
        pos = item_end

    return result


def activation_block_for_timestamp(
    chain_timestamps: list[tuple[int, int]], timestamp: int
) -> int | None:
    for block_number, block_timestamp in chain_timestamps:
        if block_timestamp >= timestamp:
            return block_number
    return None


def set_hardfork_schedule(
    evm_genesis: dict, geth_genesis: dict, chain_timestamps: list[tuple[int, int]]
) -> None:
    config = geth_genesis.get("config", {})
    if not isinstance(config, dict):
        return

    schedule: list[tuple[int, str]] = [(0, "FRONTIER")]

    def add_block_fork(field: str, fork_name: str) -> None:
        value = parse_optional_int(config.get(field))
        if value is None or value < 0:
            return
        schedule.append((value, fork_name))

    add_block_fork("homesteadBlock", "HOMESTEAD")
    add_block_fork("eip150Block", "TANGERINE")

    eip155 = parse_optional_int(config.get("eip155Block"))
    eip158 = parse_optional_int(config.get("eip158Block"))
    if eip155 is not None or eip158 is not None:
        max_spurious = max(x for x in [eip155, eip158] if x is not None)
        schedule.append((max_spurious, "SPURIOUS_DRAGON"))

    add_block_fork("byzantiumBlock", "BYZANTIUM")
    add_block_fork("constantinopleBlock", "CONSTANTINOPLE")
    add_block_fork("petersburgBlock", "PETERSBURG")
    add_block_fork("istanbulBlock", "ISTANBUL")
    add_block_fork("muirGlacierBlock", "MUIR_GLACIER")
    add_block_fork("berlinBlock", "BERLIN")
    add_block_fork("londonBlock", "LONDON")
    add_block_fork("arrowGlacierBlock", "ARROW_GLACIER")
    add_block_fork("grayGlacierBlock", "GRAY_GLACIER")
    add_block_fork("mergeNetsplitBlock", "MERGE")

    def add_time_fork(field: str, fork_name: str) -> None:
        ts = parse_optional_int(config.get(field))
        if ts is None:
            return
        activation = activation_block_for_timestamp(chain_timestamps, ts)
        if activation is not None:
            schedule.append((activation, fork_name))

    add_time_fork("shanghaiTime", "SHANGHAI")
    add_time_fork("cancunTime", "CANCUN")
    add_time_fork("pragueTime", "PRAGUE")

    schedule.sort(key=lambda item: item[0])
    deduped: list[tuple[int, str]] = []
    for block_number, fork_name in schedule:
        if deduped and deduped[-1][0] == block_number:
            deduped[-1] = (block_number, fork_name)
        else:
            deduped.append((block_number, fork_name))

    chain_spec = evm_genesis.setdefault("chain_spec", {})
    chain_spec["hardforks"] = [[block, fork] for block, fork in deduped]


def main() -> int:
    if len(sys.argv) not in (4, 5):
        print(
            "Usage: genesis_adapter.py <geth_genesis.json> <template_genesis_dir> <output_dir> [chain_rlp_path]",
            file=sys.stderr,
        )
        return 1

    input_genesis = Path(sys.argv[1])
    template_dir = Path(sys.argv[2])
    output_dir = Path(sys.argv[3])
    chain_rlp_path = Path(sys.argv[4]) if len(sys.argv) == 5 else None

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

    chain_timestamps: list[tuple[int, int]] = []
    if chain_rlp_path is not None and chain_rlp_path.exists():
        try:
            chain_timestamps = load_chain_timestamps(chain_rlp_path)
        except Exception as exc:
            print(
                f"Warning: failed to parse {chain_rlp_path} for fork schedule: {exc}",
                file=sys.stderr,
            )

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
    set_hardfork_schedule(evm_genesis, geth_genesis, chain_timestamps)
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
    contract_accounts = 0
    storage_slots = 0

    for raw_address, alloc_entry in alloc.items():
        address = checksum_address(raw_address)
        entry = alloc_entry if isinstance(alloc_entry, dict) else {}

        raw_code = entry.get("code", "0x")
        code = normalize_hex_data(raw_code, f"alloc[{raw_address}].code")
        code_hash = (
            EMPTY_CODE_HASH
            if code == "0x"
            else "0x" + keccak_256(bytes.fromhex(code[2:])).hex()
        )
        if code != "0x":
            contract_accounts += 1

        nonce = parse_maybe_int(entry.get("nonce"), 0)
        if nonce < 0:
            raise ValueError(f"Negative nonce for {raw_address}")

        storage = entry.get("storage", {})
        if storage is None:
            storage = {}
        if not isinstance(storage, dict):
            raise ValueError(f"alloc[{raw_address}].storage must be an object")

        normalized_storage = {}
        for raw_slot, raw_value in storage.items():
            slot = parse_int(raw_slot, f"alloc[{raw_address}].storage slot")
            value = parse_int(raw_value, f"alloc[{raw_address}].storage value")
            normalized_storage[to_hex_u256(slot, "storage slot")] = to_hex_u256(
                value, "storage value"
            )
        storage_slots += len(normalized_storage)

        evm_accounts.append(
            {
                "address": address,
                "code_hash": code_hash,
                "code": code,
                "nonce": nonce,
                "storage": normalized_storage,
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
        "Generated Hive genesis at "
        f"{output_dir} (chain_id={chain_id}, alloc_accounts={len(evm_accounts)}, "
        f"contracts={contract_accounts}, storage_slots={storage_slots})",
        file=sys.stderr,
    )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
