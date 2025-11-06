import * as secp from "@noble/secp256k1";
import { Signature, ethers } from "ethers";
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
    const digest = ethers.keccak256(message);
    const signatureBytes = await this.signProvider(digest);
    const signature = Signature.from(signatureBytes);
    this.cachePublicKey(digest, signature);
    return ethers.getBytes(ethers.concat([signature.r, signature.s]));
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

    let secpSig = secp.Signature.fromCompact(
      ethers.getBytesCopy(ethers.concat([signature.r, signature.s])),
    );
    secpSig = secpSig.addRecoveryBit(signature.yParity);
    const publicKey = secpSig.recoverPublicKey(ethers.getBytes(msgHash));

    this.cachedPublicKey = publicKey.toBytes(true);
  }
}
