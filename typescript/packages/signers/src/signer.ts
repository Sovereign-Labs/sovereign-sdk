/**
 * A signer interface that can be implemented to provide custom signers for use in Sovereign SDK applications.
 */
export interface Signer {
  /**
   * Sign a message.
   *
   * @param message - The message to sign
   * @returns A promise that resolves to the signature
   */
  sign(message: Uint8Array): Promise<Uint8Array>;
  /**
   * Get the public key.
   *
   * @returns A promise that resolves to the public key
   */
  publicKey(): Promise<Uint8Array>;
}

/**
 * Options for initializing a signer.
 */
export type SignerOpt = {
  /**
   * The rollup schema generated during building of the rollup.
   *
   * Can be used to serialize and deserialize types and provide human readable representation
   * of signing data.
   */
  schema: Record<string, unknown>;
  /**
   * The curve to use for signing.
   */
  curve: "ed25519" | "secp256k1";
};
