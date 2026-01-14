import type Transport from "@ledgerhq/hw-transport";
import TransportNodeHid from "@ledgerhq/hw-transport-node-hid";
import { LedgerSolanaSignerBase } from "./common";

async function loadNodeTransport(): Promise<Transport> {
  return TransportNodeHid.create();
}

export class LedgerSolanaSigner extends LedgerSolanaSignerBase {
  constructor(derivationPath = "44'/501'") {
    super(loadNodeTransport, derivationPath);
  }
}
