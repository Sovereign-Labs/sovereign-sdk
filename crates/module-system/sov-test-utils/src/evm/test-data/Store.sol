// SPDX-License-Identifier: MIT

// solc --abi --bin  Store.sol  -o . --overwrite
pragma solidity ^0.8.0;
contract SimpleStorage {
    uint256 public num;
    
    function set(uint256 _num) public {
        num = _num;
    }
    
    function get() public view returns (uint) {
        return num;
    }
}