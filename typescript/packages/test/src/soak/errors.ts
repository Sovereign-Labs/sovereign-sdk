export class NotImplementedError extends Error {
  constructor(symbol: string) {
    super(`Not implemented: ${symbol}`);
    this.name = "NotImplementedError";
  }
}
