// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

contract Create2Child {
    uint256 public immutable initValue;

    constructor(uint256 _initValue) {
        initValue = _initValue;
    }
}

contract DelegateTarget {
    function writeDelegated(uint256 value) external {
        assembly {
            sstore(0, value)
        }
    }
}

contract CallReceiver {
    event Paid(address indexed from, uint256 amount);

    receive() external payable {
        emit Paid(msg.sender, msg.value);
    }

    function getBalance() external view returns (uint256) {
        return address(this).balance;
    }
}

contract KitchenSink {
    struct DynamicStruct {
        uint256 id;
        string label;
        bytes blob;
        bool enabled;
    }

    error CustomFailure(uint256 code, address sender);

    event ComplexEvent(address indexed sender, uint256 indexed indexedValue, string message, bytes payload);
    event SecondaryEvent(uint256 indexed sequence, address target, uint256 amount);
    event MappingWritten(address indexed account, uint256 value);
    event NestedWritten(address indexed account, uint256 indexed key, bytes32 value);
    event Received(address indexed from, uint256 amount);
    event EtherForwarded(address indexed to, uint256 amount, bool success);
    event DelegateWrite(uint256 value);
    event Create2Deployed(address indexed expected, address indexed deployed);

    uint256 public delegatedValue;
    uint256 public simpleValue;
    int256 public signedValue;
    address public storedAddress;
    bool public storedFlag;

    mapping(address => uint256) public simpleMapping;
    mapping(address => mapping(uint256 => bytes32)) private nestedMapping;

    bytes private storedBytes;
    string private storedString;
    uint256[] private storedArray;
    DynamicStruct private storedStruct;

    receive() external payable {
        emit Received(msg.sender, msg.value);
    }

    function setSimpleValue(uint256 value) external {
        simpleValue = value;
    }

    function setSimpleMapping(address account, uint256 value) external {
        simpleMapping[account] = value;
        emit MappingWritten(account, value);
    }

    function setNestedMapping(address account, uint256 key, bytes32 value) external {
        nestedMapping[account][key] = value;
        emit NestedWritten(account, key, value);
    }

    function getNestedMapping(address account, uint256 key) external view returns (bytes32) {
        return nestedMapping[account][key];
    }

    function setScalarBundle(uint256 unsignedVal, int256 signedVal, address addr, bool flag) external {
        simpleValue = unsignedVal;
        signedValue = signedVal;
        storedAddress = addr;
        storedFlag = flag;
    }

    function getScalarBundle() external view returns (uint256, int256, address, bool) {
        return (simpleValue, signedValue, storedAddress, storedFlag);
    }

    function setDynamicBundle(
        bytes calldata rawBytes,
        string calldata rawString,
        uint256[] calldata values,
        DynamicStruct calldata dynamicStruct
    ) external {
        storedBytes = rawBytes;
        storedString = rawString;
        storedArray = values;
        storedStruct = dynamicStruct;
    }

    function getDynamicBundle() external view returns (bytes memory, string memory, uint256[] memory, DynamicStruct memory) {
        return (storedBytes, storedString, storedArray, storedStruct);
    }

    function emitMultipleEvents(uint256 indexedValue, string calldata message, bytes calldata payload) external {
        emit ComplexEvent(msg.sender, indexedValue, message, payload);
        emit SecondaryEvent(indexedValue + 1, address(this), payload.length);
    }

    function revertWithString() external pure {
        revert("KitchenSink:string-revert");
    }

    function revertWithCustom(uint256 code) external view {
        revert CustomFailure(code, msg.sender);
    }

    function revertWithPanic() external pure {
        assert(false);
    }

    function forwardEther(address payable to, uint256 amount) external returns (bool) {
        (bool ok, ) = to.call{value: amount}("");
        require(ok, "KitchenSink:forward-failed");
        emit EtherForwarded(to, amount, ok);
        return ok;
    }

    function deployCreate2(bytes32 salt, uint256 childValue) external returns (address deployed, address expected) {
        bytes memory bytecode = abi.encodePacked(type(Create2Child).creationCode, abi.encode(childValue));
        bytes32 codeHash = keccak256(bytecode);
        expected = computeCreate2Address(salt, codeHash);

        assembly {
            deployed := create2(0, add(bytecode, 0x20), mload(bytecode), salt)
        }

        require(deployed != address(0), "KitchenSink:create2-failed");
        require(deployed == expected, "KitchenSink:create2-mismatch");

        emit Create2Deployed(expected, deployed);
    }

    function computeCreate2Address(bytes32 salt, bytes32 codeHash) public view returns (address) {
        bytes32 digest = keccak256(abi.encodePacked(bytes1(0xff), address(this), salt, codeHash));
        return address(uint160(uint256(digest)));
    }

    function delegateWrite(address implementation, uint256 value) external returns (bool ok, bytes memory result) {
        (ok, result) = implementation.delegatecall(abi.encodeWithSignature("writeDelegated(uint256)", value));
        require(ok, "KitchenSink:delegate-failed");
        emit DelegateWrite(delegatedValue);
    }

    function hashAndRoundtrip(uint256 value, string calldata text, bytes calldata data)
        external
        pure
        returns (bytes32 digest, uint256 outValue, string memory outText, bytes memory outData)
    {
        bytes memory encoded = abi.encode(value, text, data);
        digest = keccak256(encoded);
        (outValue, outText, outData) = abi.decode(encoded, (uint256, string, bytes));
    }

    function encodePackedHash(address account, uint256 value, bytes calldata data) external pure returns (bytes32) {
        return keccak256(abi.encodePacked(account, value, data));
    }

    function precompileSha256(bytes calldata data) external pure returns (bytes32) {
        return sha256(data);
    }

    function precompileRipemd160(bytes calldata data) external pure returns (bytes20) {
        return ripemd160(data);
    }

    function precompileEcrecover(bytes32 digest, uint8 v, bytes32 r, bytes32 s) external pure returns (address) {
        return ecrecover(digest, v, r, s);
    }
}
