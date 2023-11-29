1. Switch to the `examples/demo-rollup` directory (which is where this `README.md` is located!), and compile the application:

```shell,test-ci
$ cd examples/demo-rollup/
$ cargo build --bins
```

2.
```sh,test-ci
$ make clean-rollup-db2
```


3.
```sh,test-ci,bashtestmd:long-running
$ cargo run
```

4.
```sh,test-ci
$ make test-create-token
```

5.

```bash,test-ci,bashtestmd:compare-output
$ cargo run --bin sov-cli -- transactions import from-file bank --path ../test-data/requests/transfer.json
Adding the following transaction to batch:
{
  "bank": {
    "Transfer": {
      "to": "sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94",
      "coins": {
        "amount": 200,
        "token_address": "sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"
      }
    }
  }
}
```

6.

```bash,test-ci
$ cargo run --bin sov-cli rpc submit-batch by-address sov1l6n2cku82yfqld30lanm2nfw43n2auc8clw7r5u5m6s7p8jrm4zqrr8r94
```

7.

```bash,test-ci,bashtestmd:compare-output
$ curl -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"bank_supplyOf","params":["sov1zdwj8thgev2u3yyrrlekmvtsz4av4tp3m7dm5mx5peejnesga27svq9m72"],"id":1}' http://127.0.0.1:12345
{"jsonrpc":"2.0","result":{"amount":1000},"id":1}
```