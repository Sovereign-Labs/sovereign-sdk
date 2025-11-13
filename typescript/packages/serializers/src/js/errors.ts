export class SerializerError extends Error {
  constructor(message: string) {
    super(message);
    this.name = this.constructor.name;
  }
}

export class ByteDisplayError extends SerializerError {}
