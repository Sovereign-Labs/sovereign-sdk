// SPDX-License-Identifier: MIT

pragma solidity ^0.8.0;
contract SimpleStorage {
    uint256 public num;

     event SimpleLog(
        address indexed sender,
        uint256 indexed topic1,
        uint256 indexed topic2,
        uint256 value
    );
    
    function set(uint256 _num) public {
        num = _num;
        emit SimpleLog(msg.sender, num, num, num);
    }
    
    function get() public view returns (uint) {
        return num;
    }

    function inc() public returns (uint) {
        num += 1;
        return num;
    }

    function alwaysRevert() external pure {
        revert("This function always reverts!");
    }

    function emitLogs(uint256 topic1, uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            emit SimpleLog(msg.sender, topic1, i, num);
        }
    }
}
