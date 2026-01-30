export class BorshWriter {
  private buffer: number[] = [];

  writeU8(value: number): void {
    if (value < 0 || value > 255 || !Number.isInteger(value)) {
      throw new Error(`Invalid u8 value: ${value}`);
    }
    this.buffer.push(value);
  }

  writeI8(value: number): void {
    if (value < -128 || value > 127 || !Number.isInteger(value)) {
      throw new Error(`Invalid i8 value: ${value}`);
    }
    this.buffer.push(value < 0 ? value + 256 : value);
  }

  writeU16(value: number): void {
    if (value < 0 || value > 65535 || !Number.isInteger(value)) {
      throw new Error(`Invalid u16 value: ${value}`);
    }
    this.buffer.push(value & 0xff);
    this.buffer.push((value >>> 8) & 0xff);
  }

  writeI16(value: number): void {
    if (value < -32768 || value > 32767 || !Number.isInteger(value)) {
      throw new Error(`Invalid i16 value: ${value}`);
    }
    const unsigned = value < 0 ? value + 65536 : value;
    this.writeU16(unsigned);
  }

  writeU32(value: number): void {
    if (value < 0 || value > 4294967295 || !Number.isInteger(value)) {
      throw new Error(`Invalid u32 value: ${value}`);
    }
    this.buffer.push(value & 0xff);
    this.buffer.push((value >>> 8) & 0xff);
    this.buffer.push((value >>> 16) & 0xff);
    this.buffer.push((value >>> 24) & 0xff);
  }

  writeI32(value: number): void {
    if (value < -2147483648 || value > 2147483647 || !Number.isInteger(value)) {
      throw new Error(`Invalid i32 value: ${value}`);
    }
    const unsigned = value < 0 ? value + 4294967296 : value;
    this.writeU32(unsigned);
  }

  writeU64(value: bigint | number): void {
    const bigValue = typeof value === "number" ? BigInt(value) : value;
    if (bigValue < BigInt(0) || bigValue > BigInt("0xffffffffffffffff")) {
      throw new Error(`Invalid u64 value: ${bigValue}`);
    }

    for (let i = 0; i < 8; i++) {
      this.buffer.push(Number((bigValue >> BigInt(i * 8)) & BigInt("0xff")));
    }
  }

  writeI64(value: bigint | number): void {
    const bigValue = typeof value === "number" ? BigInt(value) : value;
    if (
      bigValue < -BigInt("0x8000000000000000") ||
      bigValue > BigInt("0x7fffffffffffffff")
    ) {
      throw new Error(`Invalid i64 value: ${bigValue}`);
    }

    const unsigned =
      bigValue < BigInt(0)
        ? bigValue + BigInt("0x10000000000000000")
        : bigValue;
    this.writeU64(unsigned);
  }

  writeU128(value: bigint | number): void {
    const bigValue = typeof value === "number" ? BigInt(value) : value;
    if (bigValue < BigInt(0) || bigValue >= BigInt(1) << BigInt(128)) {
      throw new Error(`Invalid u128 value: ${bigValue}`);
    }

    for (let i = 0; i < 16; i++) {
      this.buffer.push(Number((bigValue >> BigInt(i * 8)) & BigInt("0xff")));
    }
  }

  writeI128(value: bigint | number): void {
    const bigValue = typeof value === "number" ? BigInt(value) : value;
    if (
      bigValue < -(BigInt(1) << BigInt(127)) ||
      bigValue >= BigInt(1) << BigInt(127)
    ) {
      throw new Error(`Invalid i128 value: ${bigValue}`);
    }

    const unsigned =
      bigValue < BigInt(0) ? bigValue + (BigInt(1) << BigInt(128)) : bigValue;
    this.writeU128(unsigned);
  }

  writeF32(value: number): void {
    if (!Number.isFinite(value)) {
      throw new Error(`Invalid f32 value: ${value}`);
    }

    const buffer = new ArrayBuffer(4);
    const view = new DataView(buffer);
    view.setFloat32(0, value, true); // little-endian

    for (let i = 0; i < 4; i++) {
      this.buffer.push(view.getUint8(i));
    }
  }

  writeF64(value: number): void {
    if (!Number.isFinite(value)) {
      throw new Error(`Invalid f64 value: ${value}`);
    }

    const buffer = new ArrayBuffer(8);
    const view = new DataView(buffer);
    view.setFloat64(0, value, true); // little-endian

    for (let i = 0; i < 8; i++) {
      this.buffer.push(view.getUint8(i));
    }
  }

  writeBool(value: boolean): void {
    this.buffer.push(value ? 1 : 0);
  }

  writeString(value: string): void {
    const utf8 = new TextEncoder().encode(value);
    this.writeU32(utf8.length);
    for (const byte of utf8) {
      this.buffer.push(byte);
    }
  }

  writeBytes(bytes: Uint8Array): void {
    for (const byte of bytes) {
      this.buffer.push(byte);
    }
  }

  writeVec<T>(items: T[], writeItem: (item: T) => void): void {
    this.writeU32(items.length);
    for (const item of items) {
      writeItem(item);
    }
  }

  toUint8Array(): Uint8Array {
    return new Uint8Array(this.buffer);
  }

  toHex(): string {
    return Array.from(this.buffer)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }
}
