
# This is still work in progress. 
Please don't use it

This package is a convenient starting point for building a rollup using the Sovereign SDK:


# How to run the sov-rollup-starter:
1. Starting the node:
If you want to run a fresh rollup remove the `rollup-starter-data` folder.
This will compile and start the rollup node:

```shell
cargo run --bin node
```


2. In another shell run:

```shell
make test-create-token
```

3. Test if token creation succeeded

```shell
make test-bank-supply-of:
```