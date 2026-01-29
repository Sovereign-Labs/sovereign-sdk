import { Pool } from "pg";
import { IndexerConfigError } from "./errors";

export type EventSchema = {
  number: number;
  key: string;
  value: Record<string, unknown>;
  module: string;
};

export type Database<T> = {
  get inner(): T;
  getLatestEventNumber(): Promise<number | null>;
  insertEvents: (events: EventSchema[]) => Promise<void>;
  disconnect: () => Promise<void>;
};

export type PostgresDatabase = Database<Pool>;

export function postgresDatabase(connectionString: string): PostgresDatabase {
  const pool = new Pool({ connectionString });

  return {
    inner: pool,
    async getLatestEventNumber(): Promise<number | null> {
      const result = await pool.query("SELECT MAX(number) FROM rollup_events;");
      return result.rows[0].max;
    },
    async insertEvents(events) {
      if (!events || events.length === 0) {
        return;
      }

      const placeholders = events
        .map((_, index) => {
          const offset = index * 4;
          return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${
            offset + 4
          })`;
        })
        .join(", ");

      const query = `
        INSERT INTO rollup_events 
          (number, key, value, module) 
        VALUES 
          ${placeholders}
        RETURNING id;
      `;

      const values = events.flatMap((event) => [
        event.number,
        event.key,
        event.value,
        event.module,
      ]);

      await pool.query(query, values);
    },
    disconnect() {
      return pool.end();
    },
  };
}

export function getDefaultDatabase(): Database<unknown> {
  if (process.env.DATABASE_URL === undefined) {
    throw new IndexerConfigError("DATABASE_URL env var not set");
  }

  return postgresDatabase(process.env.DATABASE_URL);
}
