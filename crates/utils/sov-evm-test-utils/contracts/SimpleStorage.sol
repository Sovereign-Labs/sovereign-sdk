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

    // Event with all 4 topic slots used (max allowed by EVM)
    event FullTopicLog(
        uint256 indexed topic0,
        uint256 indexed topic1,
        uint256 indexed topic2,
        uint256 data
    );

    // Event with no indexed topics (anonymous-like but named)
    event DataOnlyLog(
        uint256 value1,
        uint256 value2
    );

    // Event with only indexed params (no data payload)
    event IndexedOnlyLog(
        uint256 indexed value
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

    // Emit a single log with all 4 topic slots populated
    function emitFullTopicLog(uint256 t0, uint256 t1, uint256 t2, uint256 data) public {
        emit FullTopicLog(t0, t1, t2, data);
    }

    // Emit logs with configurable topic values for flexible testing
    function emitConfigurableLogs(
        uint256 topic1Base,
        uint256 topic2Base,
        uint256 count
    ) public {
        for (uint256 i = 0; i < count; i++) {
            emit SimpleLog(msg.sender, topic1Base + i, topic2Base + i, i);
        }
    }

    // Emit a log with no indexed topics (only event signature in topic0)
    function emitDataOnlyLog(uint256 v1, uint256 v2) public {
        emit DataOnlyLog(v1, v2);
    }

    // Emit a log with only indexed topics (data == 0x)
    function emitIndexedOnlyLog(uint256 value) public {
        emit IndexedOnlyLog(value);
    }

    // Burn gas by computing keccak256 in a loop (for gas usage testing)
    function burnGas(uint256 iterations) public pure returns (bytes32) {
        bytes32 result = keccak256(abi.encodePacked(iterations));
        for (uint256 i = 0; i < iterations; i++) {
            result = keccak256(abi.encodePacked(result));
        }
        return result;
    }
}
