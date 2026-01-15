import Solana from "@ledgerhq/hw-app-solana";
import type Transport from "@ledgerhq/hw-transport";
import type { Signer } from "../signer";

export type TransportLoader = () => Promise<Transport>;

export class LedgerSolanaSignerBase implements Signer {
  readonly __ledgerSolanaSigner = true as const;

  private transport: Transport | null = null;
  private solanaApp: Solana | null = null;

  constructor(
    private loadTransport: TransportLoader,
    private derivationPath = "44'/501'",
  ) {}

  private async connect(): Promise<void> {
    if (this.transport && this.solanaApp) {
      return;
    }

    try {
      this.transport = await this.loadTransport();

      const solana = new Solana(this.transport);
      const version = await solana.getAppConfiguration().then((r) => r.version);
      const [major, minor, _patch] = version.split(".").map(Number);
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
