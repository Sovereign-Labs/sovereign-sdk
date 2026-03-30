import type { Signer } from "@sovereign-sdk/signers";

export type PhantomPublicKey = {
  toString(): string;
  toBytes(): Uint8Array;
};

export type PhantomProvider = {
  isPhantom?: boolean;
  isConnected?: boolean;
  publicKey: PhantomPublicKey | null;
  connect: (options?: { onlyIfTrusted?: boolean }) => Promise<{
    publicKey: PhantomPublicKey;
  }>;
  disconnect: () => Promise<void>;
  signMessage: (
    message: Uint8Array,
    display?: "utf8" | "hex",
  ) => Promise<{ signature: Uint8Array }>;
};

type PhantomWindow = Window & {
  phantom?: {
    solana?: PhantomProvider;
  };
  solana?: PhantomProvider;
};

export function getPhantomProvider(win: Window = window): PhantomProvider | null {
  const phantomWindow = win as PhantomWindow;
  const provider = phantomWindow.phantom?.solana ?? phantomWindow.solana;

  if (!provider || !provider.isPhantom) {
    return null;
  }

  return provider;
}

export class PhantomSigner implements Signer {
  constructor(private readonly provider: PhantomProvider) {}

  async sign(message: Uint8Array): Promise<Uint8Array> {
    const { signature } = await this.provider.signMessage(message, "utf8");
    return new Uint8Array(signature);
  }

  async publicKey(): Promise<Uint8Array> {
    if (!this.provider.publicKey) {
      throw new Error("Phantom wallet is not connected");
    }

    return new Uint8Array(this.provider.publicKey.toBytes());
  }
}

declare global {
  interface Window {
    phantom?: {
      solana?: PhantomProvider;
    };
    solana?: PhantomProvider;
  }
}
