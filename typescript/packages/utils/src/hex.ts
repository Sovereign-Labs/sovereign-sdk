export type HexString = string;

/**
 * Converts a hexadecimal string to a Uint8Array.
 *
 * @param hex - The hexadecimal string to convert. May start with '0x'.
 * @returns A Uint8Array representing the bytes of the hex string.
 * @throws {Error} If the input contains non-hex characters or has an odd length.
 */
export function hexToBytes(hex: HexString): Uint8Array {
  const cleanHex = hex.startsWith("0x") ? hex.slice(2) : hex;

  if (!/^[\da-fA-F]*$/.test(cleanHex)) {
    throw new Error("Invalid hex string: contains non-hex characters");
  }

  if (cleanHex.length % 2 !== 0) {
    throw new Error("Invalid hex string: length must be even");
  }

  const matches = cleanHex.match(/[\da-f]{2}/gi) ?? [];

  return new Uint8Array(matches.map((h) => Number.parseInt(h, 16)));
}

/**
 * Converts a Uint8Array to a hexadecimal string.
 *
 * @param bytes - The bytes to convert.
 * @returns A hexadecimal string representation of the input bytes.
 */
export function bytesToHex(bytes: Uint8Array): HexString {
  let hex = "";

  for (let i = 0; i < bytes.length; i++) {
    let hexValue = bytes[i].toString(16);

    if (hexValue.length === 1) {
      hexValue = `0${hexValue}`;
    }

    hex += hexValue;
  }

  return hex;
}

/**
 * Ensures the input is a Uint8Array. If a hex string is provided, it is converted to bytes.
 *
 * @param input - The input value, either a hex string or Uint8Array.
 * @returns The input as a Uint8Array.
 * @throws {Error} If the input is a string but not a valid hex string.
 */
export function ensureBytes(input: HexString | Uint8Array): Uint8Array {
  if (typeof input === "string") {
    return hexToBytes(input);
  }

  return input;
}
