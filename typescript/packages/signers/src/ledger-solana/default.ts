import type Transport from "@ledgerhq/hw-transport";
import TransportWebHID from "@ledgerhq/hw-transport-webhid";
import TransportWebUSB from "@ledgerhq/hw-transport-webusb";
import { LedgerSolanaSignerBase } from "./common";

async function loadBrowserTransport(): Promise<Transport> {
  if (await TransportWebHID.isSupported()) {
    return TransportWebHID.create();
  }
  if (await TransportWebUSB.isSupported()) {
    return TransportWebUSB.create();
  }
  throw new Error("No supported Ledger transport available (WebHID or WebUSB)");
}

export class LedgerSolanaSigner extends LedgerSolanaSignerBase {
  constructor(derivationPath = "44'/501'") {
    super(loadBrowserTransport, derivationPath);
  }
}
