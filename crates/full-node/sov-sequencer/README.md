# Sov-Sequencer

Simple implementation of based sequencer generic over batch builder and DA service. See `openapi-v3.yaml` to see the documentation for its API.

### Submit transactions

Please see [`demo-rollup` README](../../examples/demo-rollup/README.md#how-to-submit-transactions).

### Publish blob

To submit transactions to DA layer, sequencer needs to publish them. This can be done by triggering `publishBatch` endpoint:

```bash
./target/debug/sov-cli publish-batch ...
```

or using plain curl:

```bash
curl -sS -X POST -H "Content-Type: application/json" --data '{"transactions": []}' http://localhost:12346/sequencer/batches
```

After some time, processed transaction should appear in logs of running rollup

### Testing

This crate uses `postgres` containers during integration testing, this requires a working and available docker setup.

These tests can be skipped by setting the `SOV_TEST_SKIP_DOCKER` env var to `1` like so:

```bash
SOV_TEST_SKIP_DOCKER=1 cargo nextest run
```

### Postgres configuration via environment variables

When running the preferred sequencer with Postgres, you can supply the connection string via environment variables without storing secrets in your config file.

Precedence (first non-empty wins):

1. `SOV_SEQUENCER_POSTGRES_URL`
2. `DATABASE_URL`
3. `sequencer.preferred.postgres_connection_string` in your TOML config

Example:

```bash
SOV_SEQUENCER_POSTGRES_URL="postgres://user:pass@host:5432/db" cargo run -p sov-sequencer -- ...
```
