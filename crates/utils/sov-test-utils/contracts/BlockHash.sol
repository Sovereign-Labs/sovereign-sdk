// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;
contract BlockHash {
    function block_hash(uint number) public view returns (bytes32) {
        return blockhash(number);
    }
}
