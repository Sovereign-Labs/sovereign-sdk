export type DeepPartial<T> = T extends unknown[]
  ? T
  : T extends Record<string, unknown>
    ? {
        [P in keyof T]?: DeepPartial<T[P]>;
      }
    : T;
