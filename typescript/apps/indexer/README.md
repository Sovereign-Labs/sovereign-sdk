# @sovereign-sdk/indexer

An extendable indexer for Sovereign SDK rollups written in TypeScript.

## Running

With your database accessible & rollup running the indexer can be started like so:

```bash
DATABASE_URL="postgres://YOUR_DB_STRING" npx @sovereign-sdk/indexer --rollup-url http://localhost:12346
```

## Database Setup

The indexer expects the database to have the events table structure as defined in `db/create_events_table.sql`. This table is used to store rollup events with their associated metadata, timestamps, and payload information.

Currently this package doesn't perform migration management or have any extra opinions about database schema so it can potentially connect to existing databases, this might change in the future though.

#### Local development

For local development of your rollup+application you might want to have the indexer run against a local postgres database, the following section provides instructions on how to do this.

#### Prerequisites

- Docker installed on your system
- Docker daemon running

#### Setup Steps

1. Pull the official Postgres Docker image:

```bash
docker pull postgres
```

2. Create and start a Postgres container:

```bash
docker run --name sov-indexer-db \
  -e POSTGRES_PASSWORD=admin123 \
  -d \
  -p 5432:5432 \
  postgres
```

> **Note**: If you change the password, make sure to update it in the `dev` npm script connection string as well.

3. Verify the container is running:

```bash
docker ps
```

4. If you need to start an existing container later:

```bash
docker start sov-indexer-db
```

5. Initialize the database schema:

```bash
docker exec -i sov-indexer-db psql -U postgres < ./db/create_events_table.sql
```

### Connection Details

- Host: `localhost`
- Port: `5432`
- Username: `postgres`
- Password: `admin123`
- Database: `postgres`

### Useful Docker Commands

Stop the container:

```bash
docker stop sov-indexer-db
```

Remove the container (will delete all data):

```bash
docker rm sov-indexer-db
```

View container logs:

```bash
docker logs sov-indexer-db
```

