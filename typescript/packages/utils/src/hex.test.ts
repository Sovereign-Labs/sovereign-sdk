import { describe, expect, it } from "vitest";
import { bytesToHex, ensureBytes, hexToBytes } from "./hex";

describe("hexToBytes", () => {
  it("should convert a valid hex string to Uint8Array", () => {
    const hex = "0a1b2c";
    const result = hexToBytes(hex);
    expect(result).toEqual(new Uint8Array([10, 27, 44]));
  });

  it("should handle hex strings with 0x prefix", () => {
    const hex = "0x0a1b2c";
    const result = hexToBytes(hex);
    expect(result).toEqual(new Uint8Array([10, 27, 44]));
  });

  it("should throw if hex string contains non-hex characters", () => {
    expect(() => hexToBytes("0x0g")).toThrow(
      "Invalid hex string: contains non-hex characters",
    );
  });

  it("should throw if hex string has odd length", () => {
    expect(() => hexToBytes("abc")).toThrow(
      "Invalid hex string: length must be even",
    );
  });
});

describe("bytesToHex", () => {
  it("should convert Uint8Array to hex string", () => {
    const arr = new Uint8Array([10, 27, 44]);
    const result = bytesToHex(arr);
    expect(result).toBe("0a1b2c");
  });
  it("should convert zero array", () => {
    const arr = new Uint8Array([0]);
    const result = bytesToHex(arr);
    expect(result).toBe("00");
  });
});

describe("ensureBytes", () => {
  it("should return Uint8Array if input is Uint8Array", () => {
    const arr = new Uint8Array([1, 2, 3]);
    expect(ensureBytes(arr)).toBe(arr);
  });

  it("should convert hex string to Uint8Array", () => {
    const hex = "0a1b2c";
    expect(ensureBytes(hex)).toEqual(new Uint8Array([10, 27, 44]));
  });

  it("should throw if hex string is invalid", () => {
    expect(() => ensureBytes("xyz")).toThrow();
  });
});
