import type { Signer } from "./signer";

/** Adapts a raw Ed25519 signer to Commonware's 0x00-prefixed signing format. */
export class CommonwareSigner implements Signer {
  constructor(private readonly signer: Signer) {}

  sign(message: Uint8Array): Promise<Uint8Array> {
    const signingBytes = new Uint8Array(message.length + 1);
    signingBytes.set(message, 1);
    return this.signer.sign(signingBytes);
  }

  publicKey(): Promise<Uint8Array> {
    return this.signer.publicKey();
  }
}
