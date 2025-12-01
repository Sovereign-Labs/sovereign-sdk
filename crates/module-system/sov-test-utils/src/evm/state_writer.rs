//! //Multisig Wallet Contract
//! //SPDX-License-Identifier: MIT
//! pragma solidity ^0.8.18;
//!
//! contract Tester {
//!     event Test(uint256 timestamp, uint256 blocknumber, uint256 start, uint256 numCells, uint256 value);
//!     mapping(uint256 => uint256) slots;
//!
//!     function testWriteValuesAt(uint256 start, uint256 numCells, uint256 value) public returns (uint256) {
//!         for(uint i = 0; i < numCells; i++) {
//!             slots[i] = value;
//!         }
//!         emit Test(block.timestamp, block.number, start, numCells, value);
//!         return value;
//!     }
//! }
use alloy_sol_types::sol;

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "StateWriter.bin")))]
    StateWriter,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "StateWriter.abi"
    )
);
