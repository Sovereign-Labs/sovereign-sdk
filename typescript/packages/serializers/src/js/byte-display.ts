import { bech32, bech32m } from "bech32";
import type { ByteDisplay } from "./types";
import { hexToBytes } from "@sovereign-sdk/utils";
import { ByteDisplayError } from "./errors";
import bs58 from "bs58";

export function parse(display: ByteDisplay, input: string): number[] {
  if (display === "Hex") {
    return Array.from(hexToBytes(input));
  }

  if (display === "Base58") {
    return Array.from(bs58.decode(input));
  }

  if (display === "Decimal") {
    const parsed = input.replace("[", "").replace("]", "");
    return parsed.split(",").map((s) => parseInt(s.trim()));
  }

  if (typeof display === "object" && "Bech32" in display) {
    try {
      const decoded = bech32.decode(input);

      // Verify the HRP matches
      if (decoded.prefix !== display.Bech32.prefix) {
        throw new ByteDisplayError(
          `Expected prefix '${display.Bech32.prefix}' but got '${decoded.prefix}'`
        );
      }

      // Convert from 5-bit words to 8-bit bytes
      const bytes = bech32.fromWords(decoded.words);
      return bytes;
    } catch (error) {
      throw new ByteDisplayError(
        `Failed to decode bech32: ${(error as any).message}`
      );
    }
  }

  if (typeof display === "object" && "Bech32m" in display) {
    try {
      const decoded = bech32m.decode(input);

      // Verify the HRP matches
      if (decoded.prefix !== display.Bech32m.prefix) {
        throw new ByteDisplayError(
          `Expected prefix '${display.Bech32m.prefix}' but got '${decoded.prefix}'`
        );
      }

      // Convert from 5-bit words to 8-bit bytes
      const bytes = bech32m.fromWords(decoded.words);
      return bytes;
    } catch (error) {
      throw new ByteDisplayError(
        `Failed to decode bech32m: ${(error as any).message}`
      );
    }
  }

  throw new ByteDisplayError(`Unsupported display: ${display}`);
}
