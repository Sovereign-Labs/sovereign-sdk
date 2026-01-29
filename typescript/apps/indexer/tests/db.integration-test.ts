import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  PostgreSqlContainer,
  type StartedPostgreSqlContainer,
} from "@testcontainers/postgresql";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import {
  type EventSchema,
  type PostgresDatabase,
  postgresDatabase,
} from "../src/db";

function getTestEvent(number: number): EventSchema {
  return {
    key: "Bank/TokenTransfer",
    value: { data: "stuff" },
    module: "Bank",
    number,
  };
}

describe("Postgres queries", () => {
  vi.setConfig({ hookTimeout: 60000 });

  let db: PostgresDatabase | undefined;
  let container: StartedPostgreSqlContainer | undefined;

  beforeAll(async () => {
    container = await new PostgreSqlContainer().start();
    db = postgresDatabase(container.getConnectionUri());
    const migration = readFileSync(
      join(__dirname, "..", "db", "create_events_table.sql"),
      "utf8",
    );
    await db.inner.query(migration);
    await db.disconnect();
    await container.snapshot();
  });

  beforeEach(async () => {
    db = postgresDatabase(container!.getConnectionUri());
  });

  afterEach(async () => {
    await db?.disconnect();
    await container?.restoreSnapshot();
  });

  afterAll(async () => {
    await container?.stop();
  });

  describe("getLatestEventNumber", () => {
    it("should return undefined if empty", async () => {
      const actual = await db?.getLatestEventNumber();
      expect(actual).toBe(null);
    });
    it("should return the heighest event number", async () => {
      const events = [
        getTestEvent(1),
        getTestEvent(2),
        getTestEvent(3),
        getTestEvent(4),
        getTestEvent(5),
        getTestEvent(6),
      ];
      await db?.insertEvents(events);

      const actual = await db?.getLatestEventNumber();
      expect(actual).toBe(6);
    });
  });
});
