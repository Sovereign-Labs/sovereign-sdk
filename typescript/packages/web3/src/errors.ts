/**
 * Base error class for Sovereign SDK Web3.
 */
export class SovereignError extends Error {
  constructor(message: string) {
    super(message);

    this.name = this.constructor.name;
    this.stack = new Error().stack;
  }
}

export class InvalidRollupConfigError extends SovereignError {}

export class VersionMismatchError extends SovereignError {
  public readonly newVersion: string;
  public readonly currentVersion: string;
  public readonly retryable: boolean;

  constructor(message: string, newVersion: string, currentVersion: string) {
    super(message);

    this.newVersion = newVersion;
    this.currentVersion = currentVersion;
    this.retryable = newVersion !== currentVersion;
  }
}

export class SchemaError extends SovereignError {
  public readonly schema: object;
  public readonly reason: string;

  constructor(message: string, reason: string, schema: object) {
    super(`${message}: ${reason}`);

    this.reason = reason;
    this.schema = schema;
  }
}
