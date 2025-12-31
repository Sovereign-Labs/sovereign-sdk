import Solana from "@ledgerhq/hw-app-solana";
import type Transport from "@ledgerhq/hw-transport";
import type { Signer } from "./signer";

export class LedgerSolanaSigner implements Signer {
  private transport: Transport | null = null;
  private solanaApp: Solana | null = null;
  private derivationPath: string;

  constructor(derivationPath = "44'/501'") {
    this.derivationPath = derivationPath;
  }

  private async connect(): Promise<void> {
    if (this.transport && this.solanaApp) {
      return;
    }

    try {
      if (typeof window !== "undefined") {
        // Browser environment - use WebHID or WebUSB
        const TransportWebHID = (await import("@ledgerhq/hw-transport-webhid"))
          .default;
        const TransportWebUSB = (await import("@ledgerhq/hw-transport-webusb"))
          .default;

        if (await TransportWebHID.isSupported()) {
          this.transport = await TransportWebHID.create();
        } else if (await TransportWebUSB.isSupported()) {
          this.transport = await TransportWebUSB.create();
        } else {
          throw new Error(
            "No supported Ledger transport available (WebHID or WebUSB)",
          );
        }
      } else {
        // Assuming node environment
        try {
          const TransportNodeHid = (
            await import("@ledgerhq/hw-transport-node-hid")
          ).default;
          this.transport = await TransportNodeHid.create();
        } catch (e) {
          throw new Error(
            "Failed to connect via Node HID transport. Make sure your Ledger is connected.",
          );
        }
      }

      const solana = new Solana(this.transport);
      const version = await solana.getAppConfiguration().then((r) => r.version);
      const [major, minor, patch] = version.split(".").map(Number);
      if (major < 1 || (major === 1 && minor < 8)) {
        throw new Error(
          "Signing off-chain messages requires Solana Ledger App 1.8.0 or later",
        );
      }
      this.solanaApp = solana;
    } catch (error) {
      throw new Error(`Failed to connect to Ledger device: ${error}`);
    }
  }

  public async sign(message: Uint8Array): Promise<Uint8Array> {
    await this.connect();

    if (!this.solanaApp) {
      throw new Error("Ledger Solana app not initialized");
    }

    try {
      const result = await this.solanaApp.signOffchainMessage(
        this.derivationPath,
        Buffer.from(message),
      );
      return new Uint8Array(result.signature);
    } catch (error) {
      throw new Error(`Failed to sign message with Ledger: ${error}`);
    }
  }

  public async publicKey(): Promise<Uint8Array> {
    await this.connect();

    if (!this.solanaApp) {
      throw new Error("Ledger Solana app not initialized");
    }

    try {
      const result = await this.solanaApp.getAddress(
        this.derivationPath,
        false,
      );
      return new Uint8Array(result.address);
    } catch (error) {
      throw new Error(`Failed to get public key from Ledger: ${error}`);
    }
  }

  public async disconnect(): Promise<void> {
    if (this.transport) {
      await this.transport.close();
      this.transport = null;
      this.solanaApp = null;
    }
  }
}
