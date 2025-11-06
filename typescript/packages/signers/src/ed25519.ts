import * as ed from "@noble/ed25519";
import type { Signer } from "./signer";

export class Ed25519Signer implements Signer {
  private readonly privateKey: ed.Hex;

  constructor(privateKey: ed.Hex) {
    this.privateKey = privateKey;
  }

  public async sign(message: Uint8Array): Promise<Uint8Array> {
    return ed.signAsync(message, this.privateKey);
  }

  async publicKey() {
    return ed.getPublicKeyAsync(this.privateKey);
  }
}
