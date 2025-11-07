import { describe, expect, it } from "vitest";
import { SignerError } from "./errors";

describe("SignerError", () => {
  it("should expose signerId as a property", () => {
    const error = new SignerError("test message", "TestSigner");
    expect(error.signerId).toBe("TestSigner");
  });
  it("should include signerId in error message", () => {
    const error = new SignerError("test message", "TestSigner");
    expect(error.message).toBe("[TestSigner] test message");
  });
  it("should set correct error name", () => {
    const error = new SignerError("test message", "TestSigner");
    expect(error.name).toBe("SignerError");
  });
  it("should maintain stack trace", () => {
    const error = new SignerError("test message", "TestSigner");
    expect(error.stack).toBeDefined();
    expect(error.stack).toContain("Error");
  });
});
