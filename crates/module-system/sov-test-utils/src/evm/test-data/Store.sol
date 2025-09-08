// SPDX-License-Identifier: MIT

// solc --abi --bin  Store.sol  -o . --overwrite
pragma solidity ^0.8.0;
contract SimpleStorage {
    uint256 public num;

    event SimpleLog1(
        address indexed addr,    
        string indexed note,
    );

     event SimpleLog2(
        address indexed addr,    
        uint256 indexed value,
    );
    
    function set(uint256 _num) public {
        num = _num;
    }
    
    function get() public view returns (uint) {
        return num;
    }

    function alwaysRevert() external pure {
        revert("This function always reverts!");
    }

    function emitLog1(string calldata _note) external pure {
        emit SimpleLog(msg.sender, _note);
    }

    function emitLog2() external pure {
        emit SimpleLo2(msg.sender, _num);
    }
}