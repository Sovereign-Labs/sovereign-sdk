/**
 * An error that can be thrown by a signer.
 */
export class SignerError extends Error {
  public readonly signerId: string;

  /**
   * Create a new SignerError.
   *
   * @param message - The error message
   * @param signerId - The ID of the signer that threw the error
   */
  constructor(message: string, signerId: string) {
    super(`[${signerId}] ${message}`);

    this.signerId = signerId;
    this.name = this.constructor.name;
    this.stack = new Error().stack;
  }
}
