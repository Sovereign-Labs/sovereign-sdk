import type { Signer } from "../signer";

/**
 * Type guard to check if a signer is a Ledger Solana signer.
 * This allows checking the signer type without importing Ledger-specific dependencies.
 */
export function isLedgerSolanaSigner(
  signer: Signer,
): signer is Signer & { readonly __ledgerSolanaSigner: true } {
  return (
    "__ledgerSolanaSigner" in signer && signer.__ledgerSolanaSigner === true
  );
}
