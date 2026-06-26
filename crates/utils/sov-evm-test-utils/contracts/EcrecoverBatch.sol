// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title EcrecoverBatch
/// @notice Recovers a batch of ECDSA signatures via the `ecrecover` precompile (0x01).
/// Used to benchmark signature-verification cost in the rollup EVM. The recovered
/// addresses are XOR-accumulated and returned so the optimizer cannot elide the calls.
contract EcrecoverBatch {
    function recoverBatch(
        bytes32[] calldata h,
        uint8[] calldata v,
        bytes32[] calldata r,
        bytes32[] calldata s
    ) external pure returns (address acc) {
        for (uint256 i = 0; i < h.length; i++) {
            acc = address(uint160(acc) ^ uint160(ecrecover(h[i], v[i], r[i], s[i])));
        }
    }
}
