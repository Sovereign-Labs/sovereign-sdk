import { SovereignClient } from "@sovereign-sdk/web3";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Indexer } from "./indexer";

describe("Indexer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe("isRollupHealthy", () => {
    it("should be true by default", () => {
      const indexer = new Indexer({} as any);
      expect((indexer as any).isRollupHealthy).toBe(true);
    });
  });
  describe("getNextEventNumber", () => {
    it("should return 0 if there's no events in database", async () => {
      const indexer = new Indexer({
        database: {
          getLatestEventNumber: vi.fn().mockResolvedValueOnce(null),
        },
      } as any) as any;

      await expect(indexer.getNextEventNumber()).resolves.toBe(0);
    });
    it("should return database latest event number + 1", async () => {
      const indexer = new Indexer({
        database: {
          getLatestEventNumber: vi.fn().mockResolvedValueOnce(5),
        },
      } as any) as any;

      await expect(indexer.getNextEventNumber()).resolves.toBe(6);
    });
  });
  describe("setAndCheckHealth", () => {
    it("should set isRollupHealthy flag to false for connection error", () => {
      const indexer = new Indexer({} as any) as any;

      indexer.setAndCheckHealth(new SovereignClient.APIConnectionError({}));
      expect(indexer.isRollupHealthy).toBe(false);
    });
    it("should set isRollupHealthy flag to true if not connection error", () => {
      const indexer = new Indexer({} as any) as any;

      indexer.setAndCheckHealth(new Error());
      expect(indexer.isRollupHealthy).toBe(true);
    });
  });
  describe("handleRollupOffline", () => {
    it("should resolve when rollup comes online", async () => {
      const rollup = {
        healthcheck: vi
          .fn()
          .mockResolvedValueOnce(false)
          .mockResolvedValueOnce(true),
      };
      const indexer = new Indexer({ rollup: rollup as any } as any) as any;
      indexer.isRollupHealthy = false;

      const promise = indexer.handleRollupOffline();

      await vi.runAllTimersAsync();
      await vi.runAllTimersAsync();
      await promise;

      expect(indexer.rollup.healthcheck).toHaveBeenCalledTimes(2);
      expect(indexer.isRollupHealthy).toBe(true);
    });
    it("should continue checking until rollup is healthy", async () => {
      const rollup = {
        healthcheck: vi
          .fn()
          .mockResolvedValueOnce(false)
          .mockResolvedValueOnce(false)
          .mockResolvedValueOnce(false)
          .mockResolvedValueOnce(false)
          .mockResolvedValueOnce(true),
      };
      const indexer = new Indexer({ rollup: rollup as any } as any) as any;
      indexer.isRollupHealthy = false;

      const promise = indexer.handleRollupOffline();

      for (let i = 0; i < 5; i++) {
        await vi.runAllTimersAsync();
      }

      await promise;

      expect(indexer.rollup.healthcheck).toHaveBeenCalledTimes(5);
      expect(indexer.isRollupHealthy).toBe(true);
    });
  });
});
