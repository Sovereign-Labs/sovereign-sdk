import * as secp from "@noble/secp256k1";
import { hexToBytes } from "@sovereign-sdk/utils";
import { type Signature, keccak256, parseSignature } from "viem";
import { SignerError } from "./errors";
import type { Signer } from "./signer";

/** Minimal EIP-1193 provider interface (exposed by Privy wallets). */
export type EthereumProvider = {
  request: (args: { method: string; params?: unknown[] }) => Promise<unknown>;
};

export class PrivySignerError extends SignerError {
  constructor(message: string) {
    super(message, "Privy");
  }
}

export class PrivySigner implements Signer {
  private readonly provider: EthereumProvider;
  private cachedPublicKey?: Uint8Array;

  constructor(provider: EthereumProvider) {
    this.provider = provider;
  }

  private async signProvider(messageHash: string): Promise<string> {
    const signatureHex = await this.provider.request({
      method: "secp256k1_sign",
      params: [messageHash],
    });
    return signatureHex as string;
  }

  async sign(message: Uint8Array): Promise<Uint8Array> {
    const digest = keccak256(message);
    const signatureBytes = await this.signProvider(digest);
    const signature = parseSignature(signatureBytes as `0x${string}`);
    this.cachePublicKey(digest, signature);

    // Return compact signature (r + s, 64 bytes)
    const r = hexToBytes(signature.r.slice(2));
    const s = hexToBytes(signature.s.slice(2));
    return new Uint8Array([...r, ...s]);
  }

  /** Returns the public key in compressed form. */
  async publicKey(): Promise<Uint8Array> {
    if (!this.cachedPublicKey) {
      throw new PrivySignerError(
        "Public key was not available, you must call sign() first",
      );
    }

    return this.cachedPublicKey;
  }

  private cachePublicKey(msgHash: string, signature: Signature) {
    if (this.cachedPublicKey) return;

    const r = hexToBytes(signature.r.slice(2));
    const s = hexToBytes(signature.s.slice(2));
    let secpSig = secp.Signature.fromCompact(new Uint8Array([...r, ...s]));
    secpSig = secpSig.addRecoveryBit(signature.yParity ?? 0);

    const msgHashBytes = hexToBytes(msgHash.slice(2));
    const publicKey = secpSig.recoverPublicKey(msgHashBytes);

    this.cachedPublicKey = publicKey.toBytes(true);
  }
}
