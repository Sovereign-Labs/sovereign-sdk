import type { MetaMaskInpageProvider } from "@metamask/providers";
import * as secp from "@noble/secp256k1";
import { KnownTypeId, Schema } from "@sovereign-sdk/universal-wallet-wasm";
import { hexToBytes } from "@sovereign-sdk/utils";
import { parseSignature, type Signature } from "viem";
import { SignerError } from "../errors";
import type { Signer } from "../signer";

/**
 * EIP-712 signer implementation that uses MetaMask's eth_signTypedData_v4 method.
 *
 * This signer expects the message to be a valid UnsignedTransaction that can be
 * parsed by the schema's eip712Json() method.
 *
 * The signer uses the provided address for signing operations.
 */
export class Eip712Signer implements Signer {
  private readonly provider: MetaMaskInpageProvider;
  private readonly schema: Schema;
  private readonly address: string;
  private cachedPublicKey?: Uint8Array;
  private static readonly SIGNER_ID = "Eip712";

  constructor(
    provider: MetaMaskInpageProvider,
    schemaJson: Record<string, unknown>,
    address: string,
  ) {
    if (!address) {
      throw new SignerError("Address is required", Eip712Signer.SIGNER_ID);
    }
    this.provider = provider;
    this.schema = Schema.fromJSON(JSON.stringify(schemaJson));
    this.address = address;
  }

  /**
   * Cache the public key recovered from a signature.
   * Uses viem's Signature type which accepts high-s signatures (unlike ethers).
   */
  private cachePublicKey(signingHash: Uint8Array, signature: Signature): void {
    if (this.cachedPublicKey) return;

    // viem's parseSignature accepts high-s signatures without throwing
    const r = hexToBytes(signature.r.slice(2));
    const s = hexToBytes(signature.s.slice(2));

    let secpSig = secp.Signature.fromCompact(new Uint8Array([...r, ...s]));
    secpSig = secpSig.addRecoveryBit(signature.yParity);
    const publicKey = secpSig.recoverPublicKey(signingHash);

    this.cachedPublicKey = publicKey.toBytes(true); // compressed form
  }

  /**
   * Get the public key. Requires sign() to be called first to recover the key from a signature.
   */
  async publicKey(): Promise<Uint8Array> {
    if (!this.cachedPublicKey) {
      throw new SignerError(
        "Public key was not available, you must call sign() first",
        Eip712Signer.SIGNER_ID,
      );
    }

    return this.cachedPublicKey;
  }

  /**
   * Sign an UnsignedTransaction using EIP-712 typed data signing.
   */
  async sign(message: Uint8Array): Promise<Uint8Array> {
    const address = this.address;

    // Rollups invoke the signer with the chain hash appended, so drop the last 32 bytes of the message
    if (message.length < 32) {
      throw new SignerError(
        "Message too short, expected at least 32 bytes for chain hash",
        Eip712Signer.SIGNER_ID,
      );
    }
    const unsignedTxBytes = message.slice(0, -32);

    // Get the UnsignedTransaction type index
    const typeIndex = this.schema.knownTypeIndex(
      KnownTypeId.UnsignedTransaction,
    );

    // Generate the EIP-712 JSON from the message bytes
    let eip712Json: string;
    try {
      eip712Json = this.schema.eip712Json(typeIndex, unsignedTxBytes);
    } catch (error) {
      throw new SignerError(
        `Failed to generate EIP-712 JSON from message: ${error}`,
        Eip712Signer.SIGNER_ID,
      );
    }

    // Get the EIP-712 signing hash for public key recovery
    let signingHash: Uint8Array;
    try {
      signingHash = this.schema.eip712SigningHash(typeIndex, unsignedTxBytes);
    } catch (error) {
      throw new SignerError(
        `Failed to generate EIP-712 signing hash: ${error}`,
        Eip712Signer.SIGNER_ID,
      );
    }

    let signatureHex: string;
    try {
      signatureHex = (await this.provider.request({
        method: "eth_signTypedData_v4",
        params: [address, eip712Json],
      })) as string;
    } catch (error) {
      throw new SignerError(
        `Failed to sign with EIP-712: ${error}`,
        Eip712Signer.SIGNER_ID,
      );
    }

    // Parse the signature using viem (accepts high-s signatures unlike ethers)
    const signature = parseSignature(signatureHex);
    this.cachePublicKey(signingHash, signature);

    // Create secp256k1 signature and normalize to low-s form
    // MetaMask's eth_signTypedData_v4 may return high-s signatures for EIP-712 messages
    // (EIP-2 low-s requirement only applies to transactions, not off-chain message signing)
    const r = hexToBytes(signature.r.slice(2));
    const s = hexToBytes(signature.s.slice(2));
    let secpSig = secp.Signature.fromCompact(new Uint8Array([...r, ...s]));

    // Normalize s if it's > n/2 (convert high-s to low-s)
    if (secpSig.hasHighS()) {
      secpSig = secpSig.normalizeS();
    }

    // Return the normalized compact serialized signature (r + s, 64 bytes)
    return secpSig.toCompactRawBytes();
  }
}
