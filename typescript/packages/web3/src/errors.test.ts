import { describe, expect, it } from "vitest";
import {
  InvalidRollupConfigError,
  SovereignError,
  VersionMismatchError,
} from "./errors";

describe("errors", () => {
  describe("SovereignError", () => {
    it("should create error with correct name and message", () => {
      const error = new SovereignError("test message");
      expect(error.name).toBe("SovereignError");
      expect(error.message).toBe("test message");
      expect(error.stack).toBeDefined();
    });
  });

  describe("InvalidRollupConfigError", () => {
    it("should create error with correct name and message", () => {
      const error = new InvalidRollupConfigError("invalid config");
      expect(error.name).toBe("InvalidRollupConfigError");
      expect(error.message).toBe("invalid config");
    });
  });

  describe("VersionMismatchError", () => {
    it("should create error with correct properties when versions differ", () => {
      const error = new VersionMismatchError(
        "version mismatch",
        "2.0.0",
        "1.0.0",
      );
      expect(error.name).toBe("VersionMismatchError");
      expect(error.message).toBe("version mismatch");
      expect(error.newVersion).toBe("2.0.0");
      expect(error.currentVersion).toBe("1.0.0");
      expect(error.retryable).toBe(true);
    });

    it("should set retryable to false when versions are the same", () => {
      const error = new VersionMismatchError(
        "version mismatch",
        "1.0.0",
        "1.0.0",
      );
      expect(error.retryable).toBe(false);
    });
  });
});
