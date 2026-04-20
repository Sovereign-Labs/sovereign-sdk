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

[Toxiproxy](https://github.com/Shopify/toxiproxy) sits between the rollup and its
upstreams (Postgres + Celestia DA RPC/gRPC) so we can inject latency, timeouts, and
connection resets. The toxiproxy scripts live in [`../scripts/chaos/`](../scripts/chaos/)
and work for both baremetal and docker — baremetal is the default; docker mode is
selected via env vars.

### Setup

1. Uncomment the `toxiproxy` service in [`docker-compose.yml`](./docker-compose.yml).
2. Start it: `docker compose up -d toxiproxy`.
3. Populate the seven proxies from the host shell:
   ```bash
   LISTEN_ADDR=0.0.0.0 POSTGRES_UPSTREAM=host.docker.internal:5432 \
     ../scripts/chaos/toxi_apply_config.sh
   ```
4. Point the rollup at the proxied ports (`5433` for postgres, `26678` for celestia
   RPC, `9091` for celestia gRPC).

### Usage

Apply chaos via the scenario CLI (or the `make` shortcuts below):

```bash
# Steady DA latency on the primary rollup
make enable-chaos-std

# Compound failure: DA latency + postgres connection resets
make enable-chaos-brutal

# Or run scenarios directly:
../scripts/chaos/toxi_scenario.sh scenario P5 primary
../scripts/chaos/toxi_scenario.sh clear all
../scripts/chaos/toxi_scenario.sh list
```

See `../scripts/chaos/toxi_scenario.sh --help` for the full list of named toxics
(`rpc-latency`, `rpc-timeout`, `pg-reset`, `pg-latency`) and scenarios (`P1`–`P7`,
`R1`–`R2`).

### Troubleshooting

**Toxiproxy crashes when adding toxics to a proxy with active connections:**
```bash
make restart-toxiproxy
```
This restarts the container and re-populates all seven proxies in one shot.

Add toxics immediately after starting toxiproxy, before connections are established.
Tail logs with `docker compose logs toxiproxy`.
