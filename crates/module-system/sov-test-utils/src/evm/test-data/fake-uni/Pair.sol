// SPDX-License-Identifier: MIT
pragma solidity ^0.8.19;

import "./IERC20.sol";

/// @notice Very small UniswapV2-like pair
contract Pair {
    address public token0;
    address public token1;

    uint112 private reserve0; // uses uint112 like UniswapV2
    uint112 private reserve1;

    event Mint(address indexed sender, uint256 amount0, uint256 amount1);
    event Swap(address indexed sender, uint256 amount0In, uint256 amount1In, uint256 amount0Out, uint256 amount1Out, address indexed to);
    event Sync(uint112 reserve0, uint112 reserve1);

    constructor(address _token0, address _token1) {
        require(_token0 != _token1, "Identical tokens");
        (token0, token1) = _token0 < _token1 ? (_token0, _token1) : (_token1, _token0);
    }

    /// @notice Add liquidity by transferring tokens to this contract beforehand and then calling mint()
    function mint() external returns (uint256 amount0, uint256 amount1) {
        amount0 = IERC20(token0).balanceOf(address(this)) - reserve0;
        amount1 = IERC20(token1).balanceOf(address(this)) - reserve1;
        require(amount0 > 0 || amount1 > 0, "NO_LIQUIDITY_ADDED");

        reserve0 = uint112(reserve0 + uint112(amount0));
        reserve1 = uint112(reserve1 + uint112(amount1));

        emit Mint(msg.sender, amount0, amount1);
        emit Sync(reserve0, reserve1);
    }

    /// @notice swap: caller must have transferred input token(s) to this pair before calling swap.
    /// Provide either amount0Out or amount1Out (the other must be zero).
    /// `to` receives the output amount (or if multi-hop, next pair address).
    function swap(uint256 amount0Out, uint256 amount1Out, address to) external {
        require(amount0Out == 0 || amount1Out == 0, "Only one sided out supported");
        require(amount0Out < reserve0 && amount1Out < reserve1, "INSUFFICIENT_LIQUIDITY");

        // send out tokens
        if (amount0Out > 0) IERC20(token0).transfer(to, amount0Out);
        if (amount1Out > 0) IERC20(token1).transfer(to, amount1Out);

        // compute balances after the external transfer (router will have transferred inputs before calling swap)
        uint256 balance0 = IERC20(token0).balanceOf(address(this));
        uint256 balance1 = IERC20(token1).balanceOf(address(this));

        // determine how much input was provided
        uint256 amount0In = 0;
        uint256 amount1In = 0;

        if (balance0 > reserve0 - amount0Out) amount0In = balance0 - (reserve0 - amount0Out);
        if (balance1 > reserve1 - amount1Out) amount1In = balance1 - (reserve1 - amount1Out);

        require(amount0In > 0 || amount1In > 0, "INSUFFICIENT_INPUT_AMOUNT");

        // apply 0.3% fee (Uniswap V2 style): 997/1000
        // check invariant: (reserve0' * reserve1') >= (reserve0 * reserve1) after accounting for fee
        uint256 adjustedBalance0 = (balance0 * 1000) - (amount0In * 3);
        uint256 adjustedBalance1 = (balance1 * 1000) - (amount1In * 3);

        // old reserves multiplied by 1000
        uint256 reserve0Scaled = uint256(reserve0) * 1000;
        uint256 reserve1Scaled = uint256(reserve1) * 1000;

        require(adjustedBalance0 * adjustedBalance1 >= reserve0Scaled * reserve1Scaled, "K");
        
        // update reserves
        reserve0 = uint112(balance0);
        reserve1 = uint112(balance1);
        emit Swap(msg.sender, amount0In, amount1In, amount0Out, amount1Out, to);
        emit Sync(reserve0, reserve1);
    }

    function getReserves() external view returns (uint112, uint112) {
        return (reserve0, reserve1);
    }
}