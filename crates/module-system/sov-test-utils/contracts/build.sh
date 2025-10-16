solc --abi --bin  SimpleStorage.sol  -o artifacts --overwrite
solc --abi --bin  fake-uni/ERC20.sol  -o artifacts --overwrite
solc --abi --bin  fake-uni/Pair.sol  -o artifacts --overwrite
solc --abi --bin  fake-uni/Router.sol  -o artifacts --overwrite