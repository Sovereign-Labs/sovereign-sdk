import {
  type AccountMeta,
  Keypair,
  PublicKey,
  type PublicKeyInitData,
  type Signer,
  SystemProgram,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import * as borsh from "borsh";

/**
 * The default SPL Noop program ID used for logging messages on Solana.
 * This program is used by Hyperlane to emit events.
 */
export const DEFAULT_SPL_NOOP_PROGRAM_ID =
  "noopb9bkMVfRPU8AsbpTUg8AQkHtKwMYZiFUjNRtMmV";

function derivePda(seeds: (string | Buffer)[], programId: PublicKeyInitData) {
  const [pda] = PublicKey.findProgramAddressSync(
    seeds.map((s) => Buffer.from(s)),
    new PublicKey(programId),
  );
  return pda;
}

function deriveMailboxDispatchAuthorityPda(programId: PublicKeyInitData) {
  return derivePda(
    ["hyperlane_dispatcher", "-", "dispatch_authority"],
    programId,
  );
}

function deriveMailboxOutboxPda(mailboxProgramId: PublicKeyInitData) {
  return derivePda(["hyperlane", "-", "outbox"], mailboxProgramId);
}

function deriveMailboxDispatchedMessagePda(
  mailboxProgramId: PublicKeyInitData,
  uniqueMessageAccount: PublicKeyInitData,
) {
  return derivePda(
    [
      "hyperlane",
      "-",
      "dispatched_message",
      "-",
      new PublicKey(uniqueMessageAccount).toBuffer(),
    ],
    mailboxProgramId,
  );
}

class RegisterMessage {
  constructor(
    public destination: number,
    public embedded_user: number[],
    // Unused but required by hyperlane messages
    // Can send as a zeroed address for now
    public recipient = "0x0000000000000000000000000000000000000000000000000000000000000000",
  ) {}
}

class HyperlaneRegisterInstruction {
  instruction = 0;
  register_message: RegisterMessage;

  constructor(registerMessage: RegisterMessage) {
    this.register_message = registerMessage;
  }
}

// @ts-ignore
const SCHEMA = new Map([
  [
    RegisterMessage,
    {
      kind: "struct",
      fields: [
        ["destination", "u32"],
        ["embedded_user", [32]], // 32-byte array for Pubkey
        ["recipient", "string"],
      ],
    },
  ],
  [
    HyperlaneRegisterInstruction,
    {
      kind: "struct",
      fields: [
        ["instruction", "u8"],
        ["register_message", RegisterMessage],
      ],
    },
  ],
]);

/**
 * Program IDs required for Hyperlane Solana registration operations.
 */
export interface ProgramIds {
  /** The Hyperlane register program ID */
  register: string;
  /** The Hyperlane mailbox program ID */
  mailbox: string;
  /** Optional SPL Noop program ID. Defaults to DEFAULT_SPL_NOOP_PROGRAM_ID if not provided */
  splNoop?: string;
}

/**
 * Parameters for registering a user via Hyperlane.
 */
export interface RegisterParams {
  /** The destination chain ID where the registration message will be sent */
  destination: number;
  /** The public key of the embedded wallet, this will be used as the credential ID on the rollup & linked to the payers public address. */
  embedded_user: PublicKeyInitData;
}

/**
 * A prepared transaction ready to be sent to the Solana network.
 */
export interface PreparedTransaction {
  /** The Solana transaction containing the registration instruction */
  transaction: Transaction;
  /** The signers required to authorize the transaction */
  signers: Signer[];
}

/**
 * Client for building Hyperlane registration transactions on Solana.
 * This class provides methods to construct transactions that register users
 * via the Hyperlane interchain messaging protocol.
 */
export class HyperlaneSolanaRegister {
  programIds: Required<ProgramIds>;

  /**
   * Creates a new HyperlaneSolanaRegister instance.
   * @param programIds - The program IDs for Hyperlane register, mailbox, and optionally SPL Noop programs
   */
  constructor(programIds: ProgramIds) {
    this.programIds = {
      ...programIds,
      splNoop: programIds.splNoop ?? DEFAULT_SPL_NOOP_PROGRAM_ID,
    };
  }

  /**
   * Generates the account metadata and signers required for a registration transaction.
   * @param payer - The keypair that will pay for the transaction and sign it
   * @returns A tuple containing the account metadata array and signers array
   * @internal
   */
  keysAndSigners(payer: Keypair): [AccountMeta[], Signer[]] {
    const { mailbox, register, splNoop } = this.programIds;
    const uniqueMessageAccount = Keypair.generate();
    // The account ordering and PDAs must be exactly as below otherwise the transaction will fail.
    const keys: AccountMeta[] = [
      // mailbox program
      {
        pubkey: new PublicKey(mailbox),
        isSigner: false,
        isWritable: false,
      },
      // mailbox outbox pda
      {
        pubkey: deriveMailboxOutboxPda(mailbox),
        isSigner: false,
        isWritable: true,
      },
      // dispatch authority
      {
        pubkey: deriveMailboxDispatchAuthorityPda(register),
        isSigner: false,
        isWritable: false,
      },
      // system program
      {
        pubkey: SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
      // spl noop address
      {
        pubkey: new PublicKey(splNoop),
        isSigner: false,
        isWritable: false,
      },
      // payer
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      // unique message account
      {
        pubkey: uniqueMessageAccount.publicKey,
        isSigner: true,
        isWritable: true,
      },
      {
        pubkey: deriveMailboxDispatchedMessagePda(
          new PublicKey(mailbox),
          uniqueMessageAccount.publicKey,
        ),
        isSigner: false,
        isWritable: true,
      },
    ];
    const signers: Signer[] = [payer, uniqueMessageAccount];
    return [keys, signers];
  }

  /**
   * Builds a registration transaction for a user to be sent via Hyperlane.
   * @param user - The keypair of the user initiating the registration (payer and signer)
   * @param params - The registration parameters including destination chain and embedded user public key
   * @returns A prepared transaction with all necessary accounts and signers ready to be sent
   * @example
   * ```typescript
   * import { Connection, Keypair, sendAndConfirmTransaction } from "@solana/web3.js";
   * import { HyperlaneSolanaRegister } from "@sovereign-labs/hyperlane-solana-register";
   *
   * // Initialize the register client
   * const register = new HyperlaneSolanaRegister({
   *   mailbox: "GodWgCpbG683Zi8ii5qtXnfmGJGVWGWZ3hzzHavoAYag",
   *   register: "HX6EowhA5XwWj29iTFeqhprg1gUxHgv6RNUu4bRtUgob",
   * });
   *
   * // Build the registration transaction
   * const payer = Keypair.fromSecretKey(new Uint8Array(solanaKeypair));
   * const embeddedWallet = Keypair.generate();
   * const { transaction, signers } = register.build(payer, {
   *   destination: 5555, // Destination chain ID
   *   embedded_user: embeddedWallet.publicKey,
   * });
   *
   * // Send the transaction
   * const connection = new Connection("http://localhost:8899", "confirmed");
   * const signature = await sendAndConfirmTransaction(
   *   connection,
   *   transaction,
   *   signers,
   *   { commitment: "confirmed" }
   * );
   *
   * console.log("Registration transaction confirmed:", signature);
   * ```
   */
  build(
    user: Keypair,
    { destination, embedded_user }: RegisterParams,
  ): PreparedTransaction {
    const message = new RegisterMessage(
      destination,
      Array.from(new PublicKey(embedded_user).toBuffer()),
    );
    const instruction = new HyperlaneRegisterInstruction(message);
    const buf = borsh.serialize(SCHEMA, instruction);
    const [keys, signers] = this.keysAndSigners(user);
    const transactionInstruction = new TransactionInstruction({
      keys,
      programId: new PublicKey(this.programIds.register),
      data: Buffer.from(buf),
    });
    const transaction = new Transaction().add(transactionInstruction);
    return { transaction, signers };
  }
}
