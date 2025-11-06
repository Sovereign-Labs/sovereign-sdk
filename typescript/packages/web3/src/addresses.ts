import { type HexString, ensureBytes } from "@sovereign-sdk/utils";
import { bech32m } from "bech32";

/**
 * Encodes a public key as a bech32m address with the given human-readable part (hrp).
 *
 * @param publicKey - The public key to encode, as a hex string or Uint8Array.
 * @param hrp - The human-readable part for the bech32m address (e.g., "sov").
 * @param concatPublicKey - Optional. If provided, only the first `concatPublicKey` bytes of the public key are used.
 * @returns The bech32m-encoded address as a string.
 * @throws {Error} If the public key is not a valid hex string or Uint8Array.
 */
export function addressFromPublicKey(
  publicKey: HexString | Uint8Array,
  hrp: string,
  concatPublicKey = 28,
): string {
  let publicKeyBytes = ensureBytes(publicKey);

  if (concatPublicKey !== publicKey.length) {
    publicKeyBytes = publicKeyBytes.slice(0, concatPublicKey);
  }

  const words = bech32m.toWords(publicKeyBytes);
  return bech32m.encode(hrp, words);
}
