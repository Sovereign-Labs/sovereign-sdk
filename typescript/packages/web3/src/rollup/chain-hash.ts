import type { UnsignedTransaction } from "@sovereign-sdk/types";

export function chainHashFragment(chainHash: Uint8Array): string {
  if (chainHash.length < 8) {
    throw new Error("chain hash must contain at least 8 bytes");
  }

  return new DataView(
    chainHash.buffer,
    chainHash.byteOffset,
    chainHash.byteLength,
  )
    .getBigUint64(0, true)
    .toString();
}

export function refreshChainHashFragment<RuntimeCall>(
  unsignedTx: UnsignedTransaction<RuntimeCall>,
  chainHash: Uint8Array,
): UnsignedTransaction<RuntimeCall> {
  unsignedTx.details = {
    ...unsignedTx.details,
    chain_hash_fragment: chainHashFragment(chainHash),
  };

  return unsignedTx;
}

export function assertChainHashFragment<RuntimeCall>(
  unsignedTx: UnsignedTransaction<RuntimeCall>,
  chainHash: Uint8Array,
): void {
  const expectedFragment = chainHashFragment(chainHash);
  const actualFragment = unsignedTx.details.chain_hash_fragment;

  if (actualFragment !== expectedFragment) {
    throw new Error(
      `Cannot sign transaction: chain_hash_fragment ${actualFragment} does not match the current chain hash fragment ${expectedFragment}`,
    );
  }
}
