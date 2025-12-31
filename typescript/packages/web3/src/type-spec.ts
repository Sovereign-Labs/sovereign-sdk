/**
 * The base type specification for a rollup.
 *
 * Defines a centralised set of types that are used in the rollup.
 * This allows for a single source of truth for the types used in the rollup.
 */
export type BaseTypeSpec = {
  /**
   * The type of transaction used in the rollup.
   */
  // biome-ignore lint/suspicious/noExplicitAny: base spec, allow any as default to be overriden
  Transaction: any;

  /**
   * The type of unsigned transaction used in the rollup.
   */
  // biome-ignore lint/suspicious/noExplicitAny: base spec, allow any as default to be overriden
  UnsignedTransaction: any;

  /**
   * The type of runtime call used in the rollup.
   */
  // biome-ignore lint/suspicious/noExplicitAny: base spec, allow any as default to be overriden
  RuntimeCall: any;

  /**
   * The type of the dedup data used to deduplicate transactions.
   */
  // biome-ignore lint/suspicious/noExplicitAny: base spec, allow any as default to be overriden
  Dedup: any;
};

type OverrideType<Base, Override> = {
  [K in keyof Base]: K extends keyof Override ? Override[K] : Base[K];
};

/**
 * The type specification for a rollup.
 *
 * Extends the base type specification with concrete types.
 *
 * @example
 * ```typescript
 * type TxDetails = {
 *   maxPriorityFeeBips: number;
 *   maxFee: number;
 *   gasLimit?: bigint;
 *   chainId: number;
 * };
 *
 * type UnsignedTransaction = {
 *   runtimeMsg: Uint8Array;
 *   generation: number;
 *   details: TxDetails;
 * };
 *
 * type Transaction = {
 *   pubKey: string;
 *   signature: string;
 * } & UnsignedTransaction;
 *
 * type MyTypeSpec = RollupTypeSpec<{
 *   UnsignedTransaction: UnsignedTransaction,
 *   Transaction: Transaction,
 *   // ...
 * }>;
 *
 * // The typespec can now be used with the rollup class to provide type safety
 * // for the transaction types.
 * const rollup = new StandardRollup<MyTypeSpec>({
 *   schema: yourSchema,
 *   // ...
 * });
 * const result = await rollup.signAndSubmitTransaction({
 *   // ...
 * });
 * const tx = result.transaction; // Strongly typed as `Transaction` on your RollupTypeSpec.
 * ```
 *
 * @param T - The type specification to extend the base type specification with.
 */
export type RollupTypeSpec<T extends Partial<BaseTypeSpec>> = OverrideType<
  BaseTypeSpec,
  T
>;
