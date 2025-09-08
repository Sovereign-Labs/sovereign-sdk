// SPDX-License-Identifier: MIT

// solc --abi --bin  Store.sol  -o . --overwrite
pragma solidity ^0.8.0;
contract SimpleStorage {
    uint256 public num;

     event SimpleLog(
        address indexed addr,    
        uint256 value
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

    function emitOneLog() public {
        emit SimpleLog(msg.sender, num);
    }

    function emitTwoLogs() public {
        emit SimpleLog(msg.sender, num);
        emit SimpleLog(msg.sender, num + 1);
    }
}
