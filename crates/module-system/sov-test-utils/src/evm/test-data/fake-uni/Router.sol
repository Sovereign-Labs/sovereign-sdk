// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "./Pair.sol";

/// @notice Router that creates pairs and performs swaps (multi-hop)
contract Router {
    mapping(bytes32 => address) public getPair; // keccak(tokenA, tokenB) -> pair
    address[] public allPairs;

    event PairCreated(address indexed tokenA, address indexed tokenB, address pair);

    function createPair(address tokenA, address tokenB) external returns (address pair) {
        require(tokenA != tokenB, "IDENTICAL_ADDRESSES");
        (address token0, address token1) = tokenA < tokenB ? (tokenA, tokenB) : (tokenB, tokenA);
        bytes32 key = keccak256(abi.encodePacked(token0, token1));
        require(getPair[key] == address(0), "PAIR_EXISTS");
        Pair newPair = new Pair(token0, token1);
        pair = address(newPair);
        getPair[key] = pair;
        allPairs.push(pair);
        emit PairCreated(token0, token1, pair);
    }

    function pairFor(address tokenA, address tokenB) public view returns (address) {
        (address token0, address token1) = tokenA < tokenB ? (tokenA, tokenB) : (tokenB, tokenA);
        return getPair[keccak256(abi.encodePacked(token0, token1))];
    }

    function addLiquidity(address tokenA, address tokenB, uint256 amountA, uint256 amountB) external {
        address p = pairFor(tokenA, tokenB);
        require(p != address(0), "PAIR_NOT_EXISTS");
        IERC20(tokenA).transferFrom(msg.sender, p, amountA);
        IERC20(tokenB).transferFrom(msg.sender, p, amountB);
        Pair(p).mint();
    }

    /// @notice Get output amounts for a swap path (what users call before swapping)
    function getAmountsOut(uint256 amountIn, address[] memory path) public view returns (uint256[] memory amounts) {
        require(path.length >= 2, "INVALID_PATH");
        amounts = new uint256[](path.length);
        amounts[0] = amountIn;
        for (uint256 i = 0; i < path.length - 1; i++) {
            address p = pairFor(path[i], path[i + 1]);
            require(p != address(0), "PAIR_MISSING");
            (uint112 r0, uint112 r1) = Pair(p).getReserves();
            uint256 reserveIn  = path[i] < path[i + 1] ? uint256(r0) : uint256(r1);
            uint256 reserveOut = path[i] < path[i + 1] ? uint256(r1) : uint256(r0);
            
            // AMM formula with 0.3% fee
            uint256 amountInWithFee = amounts[i] * 997;
            amounts[i + 1] = (amountInWithFee * reserveOut) / (reserveIn * 1000 + amountInWithFee);
        }
    }

    /// @notice internal single-hop that returns amountOut and does the swap
    function _hop(address inToken, address outToken, uint256 amtIn, address dst) internal returns (uint256 amtOut) {
        address p = pairFor(inToken, outToken);
        require(p != address(0), "PAIR_MISSING");

        (uint112 r0, uint112 r1) = Pair(p).getReserves();
        uint256 reserveIn  = inToken < outToken ? uint256(r0) : uint256(r1);
        uint256 reserveOut = inToken < outToken ? uint256(r1) : uint256(r0);

        uint256 feeAmt = amtIn * 997; // 0.3% fee
        amtOut = (feeAmt * reserveOut) / (reserveIn * 1000 + feeAmt);

        if (inToken < outToken) {
            Pair(p).swap(0, amtOut, dst);
        } else {
            Pair(p).swap(amtOut, 0, dst);
        }
    }

    /// @notice swap exact tokens along `path` (V2-like). Caller must approve the router for path[0].
    function swapExactTokensForTokens(
        uint256 amountIn,
        uint256 amountOutMin,
        address[] calldata path,
        address to
    ) external {
        uint256 n = path.length;
        require(n >= 2, "INVALID_PATH");

        // Seed first pair with input
        address firstPair = pairFor(path[0], path[1]);
        require(firstPair != address(0), "PAIR_MISSING");
        IERC20(path[0]).transferFrom(msg.sender, firstPair, amountIn);

        uint256 amt = amountIn;

        for (uint256 i = 0; i < n - 1; ) {
            address a = path[i];
            address b = path[i + 1];

            if (i + 2 == n) {
                // last hop: send to `to` directly
                amt = _hop(a, b, amt, to);
            } else {
                // intermediate hop: send to next pair
                address nextPair = pairFor(b, path[i + 2]);
                require(nextPair != address(0), "NEXT_PAIR_MISSING");
                amt = _hop(a, b, amt, nextPair);
            }

            unchecked { ++i; }
        }

        require(amt >= amountOutMin, "INSUFFICIENT_OUTPUT_AMOUNT");
    }
}
