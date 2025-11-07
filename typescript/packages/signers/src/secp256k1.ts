import { keccak_256 } from "@noble/hashes/sha3";
import { type PrivKey, getPublicKey, signAsync } from "@noble/secp256k1";
import type { Signer } from "./signer";

export class Secp256k1Signer implements Signer {
  private readonly privateKey: PrivKey;

  constructor(privateKey: PrivKey) {
    this.privateKey = privateKey;
  }

  public async sign(message: Uint8Array): Promise<Uint8Array> {
    const msgHash = keccak_256(message);
    const signature = await signAsync(msgHash, this.privateKey);

    return signature.toCompactRawBytes();
  }

  /** Returns the public key in compressed form. */
  public async publicKey(): Promise<Uint8Array> {
    return getPublicKey(this.privateKey, true);
  }
}
