import { type Rollup, SovereignClient } from "@sovereign-sdk/web3";
import type { ErrorResponse } from "./types";

type GasTokenIdPayload = {
  token_id: string;
};

type BalancePayload = {
  amount: string;
  token_id: string;
};

type TotalSupplyPayload = {
  amount: string;
  token_id: string;
};

/**
 * Bank class for interacting with the Sovereign SDK Bank module.
 */
export class Bank {
  // biome-ignore lint/suspicious/noExplicitAny: types arent used
  private readonly rollup: Rollup<any, any>;
  private _gasTokenId?: string;

  /**
   * Creates a new Bank instance.
   *
   * @param rollup - The rollup instance to use for HTTP requests
   */
  // biome-ignore lint/suspicious/noExplicitAny: types arent used
  constructor(rollup: Rollup<any, any>) {
    this.rollup = rollup;
  }

  /**
   * Gets the balance of a specific token for a given address.
   *
   * If no token ID is provided, the gas token balance will be returned.
   *
   * @param address - The address to query the balance for
   * @param tokenId - Optional token ID. If not provided, uses the gas token
   * @returns Promise resolving to the token balance as a bigint
   * @throws {SovereignClient.APIError} When the request fails for reasons other than missing account
   * @example
   * ```typescript
   * const bank = new Bank(rollup);
   * const balance = await bank.balance("0x123...");
   * console.log(`Balance: ${balance}`);
   *
   * // Query specific token balance
   * const tokenBalance = await bank.balance("0x123...", "token_123");
   * ```
   */
  async balance(address: string, tokenId?: string): Promise<bigint> {
    const token = await this.tokenIdOrElseGasTokenId(tokenId);

    try {
      const response: BalancePayload = await this.rollup.http.get(
        `/modules/bank/tokens/${token}/balances/${address}`,
      );

      return BigInt(response.amount);
    } catch (err) {
      if (this.isMissingAccountError(err)) return BigInt(0);

      throw err;
    }
  }

  /**
   * Gets the total supply of a specific token.
   *
   * If no token ID is provided, returns the total supply of the gas token.
   *
   * @param tokenId - Optional token ID. If not provided, uses the gas token
   * @returns Promise resolving to the total supply as a bigint
   * @throws {SovereignClient.APIError} When the request fails
   * @example
   * ```typescript
   * const bank = new Bank(rollup);
   * const totalSupply = await bank.totalSupply();
   * console.log(`Total gas token supply: ${totalSupply}`);
   *
   * // Query specific token supply
   * const tokenSupply = await bank.totalSupply("token_123");
   * ```
   */
  async totalSupply(tokenId?: string): Promise<bigint> {
    const token = await this.tokenIdOrElseGasTokenId(tokenId);
    const response: TotalSupplyPayload = await this.rollup.http.get(
      `/modules/bank/tokens/${token}/total-supply`,
    );
    return BigInt(response.amount);
  }

  /**
   * Gets the gas token identifier.
   *
   * The gas token ID is cached after the first request to avoid repeated API calls.
   *
   * @returns Promise resolving to the gas token ID as a string
   * @throws {Error} When the gas token ID cannot be retrieved
   * @example
   * ```typescript
   * const bank = new Bank(rollup);
   * const gasTokenId = await bank.gasTokenId();
   * console.log(`Gas token ID: ${gasTokenId}`);
   * ```
   */
  async gasTokenId(): Promise<string> {
    if (this._gasTokenId) return this._gasTokenId;
    const response: GasTokenIdPayload = await this.rollup.http.get(
      "/modules/bank/tokens/gas_token",
    );
    this._gasTokenId = response.token_id;
    return this._gasTokenId;
  }

  private async tokenIdOrElseGasTokenId(tokenId?: string): Promise<string> {
    return tokenId ?? this.gasTokenId();
  }

  // biome-ignore lint/suspicious/noExplicitAny: todo
  private isMissingAccountError(anyError: any): boolean {
    if (anyError?.status !== 404) return false;
    const body = anyError?.error as ErrorResponse;
    return (
      body?.message?.startsWith("Balance") &&
      body?.message?.endsWith("not found")
    );
  }
}
