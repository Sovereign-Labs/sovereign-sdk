export type ErrorResponse = {
  status: number;
  message: string;
  details: Record<string, unknown>;
};
