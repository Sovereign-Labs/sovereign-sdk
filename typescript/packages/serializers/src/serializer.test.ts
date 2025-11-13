import { describe, expect, it } from "vitest";
import { convertUint8ArraysToArrays } from "./serializer";

describe("convertUint8ArraysToArrays", () => {
  it("should convert a simple Uint8Array to a regular array", () => {
    const input = new Uint8Array([1, 2, 3, 4]);
    const result = convertUint8ArraysToArrays(input);

    expect(Array.isArray(result)).toBe(true);
    expect(result).toEqual([1, 2, 3, 4]);
    expect(result).not.toBeInstanceOf(Uint8Array);
  });

  it("should handle null and undefined values", () => {
    expect(convertUint8ArraysToArrays(null)).toBe(null);
    expect(convertUint8ArraysToArrays(undefined)).toBe(undefined);
  });

  it("should handle primitive values unchanged", () => {
    expect(convertUint8ArraysToArrays(42)).toBe(42);
    expect(convertUint8ArraysToArrays("hello")).toBe("hello");
    expect(convertUint8ArraysToArrays(true)).toBe(true);
    expect(convertUint8ArraysToArrays(false)).toBe(false);
  });

  it("should convert Uint8Arrays in object properties", () => {
    const input = {
      name: "test",
      buffer: new Uint8Array([1, 2, 3]),
      count: 42,
    };

    const result = convertUint8ArraysToArrays(input);

    expect(result.name).toBe("test");
    expect(result.count).toBe(42);
    expect(Array.isArray(result.buffer)).toBe(true);
    expect(result.buffer).toEqual([1, 2, 3]);
    expect(result.buffer).not.toBeInstanceOf(Uint8Array);
  });

  it("should handle nested objects with Uint8Arrays", () => {
    const input = {
      level1: {
        level2: {
          level3: {
            deepBuffer: new Uint8Array([10, 20, 30]),
          },
        },
      },
    };

    const result = convertUint8ArraysToArrays(input);

    expect(Array.isArray(result.level1.level2.level3.deepBuffer)).toBe(true);
    expect(result.level1.level2.level3.deepBuffer).toEqual([10, 20, 30]);
  });

  it("should convert Uint8Arrays in regular arrays", () => {
    const input = [
      "string",
      42,
      new Uint8Array([1, 2]),
      new Uint8Array([3, 4, 5]),
    ];

    const result = convertUint8ArraysToArrays(input);

    expect(result[0]).toBe("string");
    expect(result[1]).toBe(42);
    expect(Array.isArray(result[2])).toBe(true);
    expect(result[2]).toEqual([1, 2]);
    expect(Array.isArray(result[3])).toBe(true);
    expect(result[3]).toEqual([3, 4, 5]);
  });

  it("should handle arrays containing objects with Uint8Arrays", () => {
    const input = [
      {
        id: 1,
        data: new Uint8Array([10, 11]),
      },
      {
        id: 2,
        data: new Uint8Array([20, 21, 22]),
      },
    ];

    const result = convertUint8ArraysToArrays(input);

    expect(result[0].id).toBe(1);
    expect(Array.isArray(result[0].data)).toBe(true);
    expect(result[0].data).toEqual([10, 11]);
    expect(result[1].id).toBe(2);
    expect(Array.isArray(result[1].data)).toBe(true);
    expect(result[1].data).toEqual([20, 21, 22]);
  });

  it("should handle complex nested structures", () => {
    const input = {
      metadata: {
        name: "complex test",
        buffers: [new Uint8Array([1, 2]), new Uint8Array([3, 4, 5])],
      },
      data: {
        items: [
          {
            id: "item1",
            payload: new Uint8Array([100, 101, 102]),
            nested: {
              moreData: new Uint8Array([200]),
            },
          },
        ],
      },
    };

    const result = convertUint8ArraysToArrays(input);

    expect(result.metadata.name).toBe("complex test");
    expect(Array.isArray(result.metadata.buffers[0])).toBe(true);
    expect(result.metadata.buffers[0]).toEqual([1, 2]);
    expect(Array.isArray(result.metadata.buffers[1])).toBe(true);
    expect(result.metadata.buffers[1]).toEqual([3, 4, 5]);
    expect(result.data.items[0].id).toBe("item1");
    expect(Array.isArray(result.data.items[0].payload)).toBe(true);
    expect(result.data.items[0].payload).toEqual([100, 101, 102]);
    expect(Array.isArray(result.data.items[0].nested.moreData)).toBe(true);
    expect(result.data.items[0].nested.moreData).toEqual([200]);
  });

  it("should handle empty Uint8Arrays", () => {
    const input = {
      emptyBuffer: new Uint8Array([]),
      normalArray: [],
      emptyBufferInArray: [new Uint8Array([])],
    };

    const result = convertUint8ArraysToArrays(input);

    expect(Array.isArray(result.emptyBuffer)).toBe(true);
    expect(result.emptyBuffer).toEqual([]);
    expect(result.emptyBuffer).toHaveLength(0);
    expect(Array.isArray(result.normalArray)).toBe(true);
    expect(result.normalArray).toEqual([]);
    expect(Array.isArray(result.emptyBufferInArray[0])).toBe(true);
    expect(result.emptyBufferInArray[0]).toEqual([]);
  });

  it("should preserve non-plain objects like Date, RegExp, and custom classes", () => {
    class CustomClass {
      constructor(public value: number) {}
    }

    const date = new Date("2023-01-01");
    const regex = /test/g;
    const customInstance = new CustomClass(42);

    const input = {
      date,
      regex,
      custom: customInstance,
      buffer: new Uint8Array([1, 2, 3]),
    };

    const result = convertUint8ArraysToArrays(input);

    expect(result.date).toBe(date);
    expect(result.regex).toBe(regex);
    expect(result.custom).toBe(customInstance);
    expect(Array.isArray(result.buffer)).toBe(true);
    expect(result.buffer).toEqual([1, 2, 3]);
  });

  it("should handle mixed array types", () => {
    const input = [
      "string",
      42,
      new Uint8Array([1, 2, 3]),
      { nested: new Uint8Array([4, 5]) },
      [new Uint8Array([6, 7, 8])],
      null,
      undefined,
    ];

    const result = convertUint8ArraysToArrays(input) as any;

    expect(result[0]).toBe("string");
    expect(result[1]).toBe(42);
    expect(Array.isArray(result[2])).toBe(true);
    expect(result[2]).toEqual([1, 2, 3]);
    expect(Array.isArray(result[3].nested)).toBe(true);
    expect(result[3]!.nested).toEqual([4, 5]);
    expect(Array.isArray(result[4][0])).toBe(true);
    expect(result[4][0]).toEqual([6, 7, 8]);
    expect(result[5]).toBe(null);
    expect(result[6]).toBe(undefined);
  });
});
