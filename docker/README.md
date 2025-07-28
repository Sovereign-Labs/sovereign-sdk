# Local Celestia setup

It consists of one validator (block maker) and an arbitrary number of bridge nodes
for rollup sequencers (1 by default).

## Example

```sh
# start the celestia network
docker compose -f docker/docker-compose.yml up --build --force-recreate -d

# grab the jwt
CELESTIA_NODE_AUTH_TOKEN="$(cat docker/celestia/credentials/bridge-0.jwt)"

# check the celestia rpc
curl -X POST \                                                                           
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer ${CELESTIA_NODE_AUTH_TOKEN}" \
  -d '{
    "id": 1,
    "jsonrpc": "2.0",
    "method": "header.GetByHeight",
    "params": [2]
  }' \
  localhost:26658

# stop the Celestia network
docker compose -f docker/docker-compose.yml down
```

### Login to GitHub registry

You'll need to be logged in to the github's registry to pull celestia images.
Follow [this guide](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry#authenticating-with-a-personal-access-token-classic)
to authorize yourself in github's container registry. (we use original celestia images which they publish in ghcr)

```shell
# this has to be ran only once, unless your token expires
$ echo $MY_PERSONAL_GITHUB_TOKEN | docker login ghcr.io -u $MY_GITHUB_USERNAME --password-stdin
```

## Multiple sequencers

To have multiple sequencers, a few conditions needs to be met:
- validator must know the number of sequencers to provision them with accounts and coins
- each sequencer must have a unique id and each id has to be a consecutive natural number
  starting from 0. (eg. 0, 1, 2)
- each sequencer other than the first one has to have the ports remapped so they don't conflict
  with other sequencers

The `docker-compose.yml` has a commented out example setup for the second sequencer. It can
be copy-pasted and adjusted for an arbitrary number of sequencers. The amount of sequencers
needs to be provided by uncommenting and aligning the `services.validator.command` field.

## Credentials

Credentials for each new sequencer are created by validator on the first startup. The validator writes
the keys and address of each sequencer to the `docker/credentials` volume. Each consecutive
run will use the same credentials until the directory is manually cleaned up.

In addition, each sequencer on startup will write it's `JWT` token to the same directory. The token is
updated during consecutive runs.

## Chaos Engineering

[Toxiproxy](https://github.com/Shopify/toxiproxy) enables chaos engineering by simulating network failures and instabilities. 
Use it to test how the rollup behaves when the connection to celestia-node is unreliable.

### Setup

1. Uncomment the toxiproxy service in [`docker-compose.yml`](./docker-compose.yml)
2. Configure your rollup to connect to port `26659` (proxied) instead of `26658` (direct)

### Usage

The proxy starts without any network toxics enabled. Use the provided scripts to control network conditions:

```bash
# Enable standard toxics (light network issues)
docker/toxiproxy/enable_standard_toxics.sh

# Enable brutal toxics (severe network issues)
docker/toxiproxy/enable_brutal_toxics.sh

# Remove all toxics (restore normal network)
docker/toxiproxy/remove_toxics.sh

# Check current toxic status
docker/toxiproxy/status_chaos.sh
```

Available toxic types include latency, timeouts, connection resets, and bandwidth limiting. 
This allows you to test rollup resilience under various network failure scenarios.

### Troubleshooting

**Toxiproxy crashes when adding toxics:**
- This happens when trying to add toxics to a proxy with active connections
- Solution: Restart toxiproxy and try again:
  ```bash
  docker compose restart toxiproxy
  # Wait a few seconds, then try adding toxics again
  ```

**Best practices:**
- Add toxics immediately after starting toxiproxy, before connections are established
- Use the remove script to clean up toxics before stopping services
- Monitor toxiproxy logs for crash indicators: `docker compose logs toxiproxy`
