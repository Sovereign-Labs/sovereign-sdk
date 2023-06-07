## Setting up SDK to run locally

* Install docker https://www.docker.com
* switch to the `demo-rollup` directory
* Start the celestia services locally
```
make clean
make start
```
* The above command should also configure your local setup so you should see some changes stashed
```
$ git status
..
..
	modified:   ../const-rollup-config/src/lib.rs
	modified:   rollup_config.toml
```
* Start the demo-rollup in a different tab
```
$ cargo +nightly run
```
* You should see the demo-rollup app consuming blocks from the docker container's celestia node
```
2023-06-07T10:03:25.473920Z  INFO jupiter::da_service: Fetching header at height=1...
2023-06-07T10:03:25.496853Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.497700Z  INFO sov_demo_rollup: Requesting data for height 2 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.497719Z  INFO jupiter::da_service: Fetching header at height=2...
2023-06-07T10:03:25.505412Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.505992Z  INFO sov_demo_rollup: Requesting data for height 3 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
2023-06-07T10:03:25.506003Z  INFO jupiter::da_service: Fetching header at height=3...
2023-06-07T10:03:25.511237Z  INFO sov_demo_rollup: Received 0 blobs
2023-06-07T10:03:25.511815Z  INFO sov_demo_rollup: Requesting data for height 4 and prev_state_root 0xa96745d3184e54d098982daf44923d84c358800bd22c1864734ccb978027a670
```
* Run the test transaction command, which creates a token
```
make test-create-token 
```
* In the tab where the demo-rollup, is running, you should shortly (in a couple of seconds) see the transaction picked up
```
2023-06-07T10:05:10.431888Z  INFO jupiter::da_service: Fetching header at height=18...
2023-06-07T10:05:20.493991Z  INFO sov_demo_rollup: Received 1 blobs
2023-06-07T10:05:20.496571Z  INFO sov_demo_rollup: receipts: BatchReceipt { batch_hash: [44, 38, 61, 124, 123, 92, 9, 196, 200, 211, 52, 149, 33, 172, 120, 239, 180, 106, 72, 9, 161, 68, 8, 87, 127, 190, 201, 94, 9, 30, 108, 188], tx_receipts: [TransactionReceipt { tx_hash: [160, 103, 81, 53, 69, 140, 72, 198, 215, 190, 38, 242, 70, 204, 226, 217, 216, 22, 210, 142, 110, 221, 222, 171, 26, 40, 158, 236, 110, 107, 160, 170], body_to_save: None, events: [], receipt: Successful }], inner: Rewarded(0) }
```
