pragma solidity ^0.8.0;

import '@openzeppelin/contracts/token/ERC20/ERC20.sol';
import "@openzeppelin/contracts/access/Ownable.sol";

contract Tether is ERC20, Ownable {
  constructor() ERC20('Tether', 'USDT') {}

  function mint(address to, uint256 amount) public onlyOwner {
    _mint(to, amount);
  }
}