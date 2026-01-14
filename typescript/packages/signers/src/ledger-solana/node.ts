import type Transport from "@ledgerhq/hw-transport";
import TransportNodeHid from "@ledgerhq/hw-transport-node-hid";
import { LedgerSolanaSignerBase } from "./common";

async function loadNodeTransport(): Promise<Transport> {
  try {
    return await TransportNodeHid.create();
  } catch (error) {
    throw new Error(
      `Failed to connect via Node HID transport. Make sure your Ledger is connected. Error: ${error}`,
    );
  }
}

export class LedgerSolanaSigner extends LedgerSolanaSignerBase {
  constructor(derivationPath = "44'/501'") {
    super(loadNodeTransport, derivationPath);
  }
}
