/// <reference path="./snap-env.d.ts" />

import type { MetaMaskInpageProvider } from "@metamask/providers";
import { bytesToHex, hexToBytes } from "@sovereign-sdk/utils";
import { SignerError } from "./errors";
import type { Signer, SignerOpt } from "./signer";

/**
 * Options for initializing a MetaMask Snap signer.
 */
export type MetaMaskSnapSignerOpt = {
  /**
   * The {@link MetaMaskInpageProvider} to use for the signer.
   *
   * If not provided, the signer will attempt to use the global `window.ethereum` provider.
   */
  provider?: MetaMaskInpageProvider;
  /**
   * The snap ID to use for the signer.
   *
   * Defaults to `npm:@sovereign-sdk/metamask-snap`.
   * Can be changed to `local:localhost:8080` for local development.
   */
  snapId?: string;
} & SignerOpt;

/**
 * Create a new MetaMask Snap signer.
 *
 * @param options - The {@link MetaMaskSnapSignerOpt} options.
 * @returns A {@link Signer} implementation utilizing Sovereign's MetaMask Snap.
 */
export const newMetaMaskSnapSigner = ({
  curve,
  schema,
  snapId = "npm:@sovereign-sdk/metamask-snap",
  ...rest
}: MetaMaskSnapSignerOpt): Signer => {
  const signerId = "MetaMaskSnap";
  const provider = rest.provider ?? window.ethereum;

  if (!provider) {
    throw new SignerError("Failed to find provider for signer", signerId);
  }

  const assertSupported = async () => {
    try {
      await provider.request({
        method: "wallet_getSnaps",
      });
    } catch {
      throw new SignerError(
        "Provider does not support MetaMask snaps",
        signerId,
      );
    }
  };

  const path = 0;

  return {
    async publicKey() {
      await assertSupported();

      const args = {
        method: "wallet_invokeSnap",
        params: {
          snapId,
          request: {
            method: "getPublicKey",
            params: { path, curve },
          },
        },
      };
      // biome-ignore lint/suspicious/noExplicitAny: todo
      const { publicKey } = (await provider.request(args)) as any;

      return hexToBytes(publicKey);
    },
    async sign(message: Uint8Array) {
      await assertSupported();

      const args = {
        method: "wallet_invokeSnap",
        params: {
          snapId,
          request: {
            method: "signTransaction",
            params: { unsignedTxHex: bytesToHex(message), schema, curve, path },
          },
        },
      };
      // biome-ignore lint/suspicious/noExplicitAny: todo
      const { signature } = (await provider.request(args)) as any;

      return hexToBytes(signature);
    },
  };
};
