# Local celestia setup

It consists of one validator (block maker) and arbitrary number of bridge nodes
for sequencers (1 by default).

## Example

```sh
# start the celestia network
docker compose -f docker/docker-compose.yml up --build --force-recreate -d

# grab the jwt
CELESTIA_NODE_AUTH_TOKEN="$(cat docker/credentials/bridge-0.jwt)"

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

# stop the celestia network
docker compose -f docker/docker-compose.yml down
```

### Login to github registry

You'll need to be logged in to the github's registry in order to pull celestia images.
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
