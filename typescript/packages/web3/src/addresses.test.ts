import { describe, expect, it } from "vitest";
import { addressFromPublicKey } from "./addresses";

describe("addressFromPublicKey", () => {
  it("should convert real example to expected human readable address", () => {
    const publicKey = Uint8Array.from([
      123, 117, 139, 242, 231, 103, 15, 175, 175, 107, 240, 1, 92, 224, 255, 90,
      168, 2, 48, 111, 199, 227, 244, 87, 98, 133, 63, 252, 55, 24, 15, 230,
    ]);
    expect(addressFromPublicKey(publicKey, "sov")).toBe(
      "sov10d6chuh8vu86ltmt7qq4ec8lt25qyvr0cl3lg4mzs5llcfnx69m",
    );
  });
});
