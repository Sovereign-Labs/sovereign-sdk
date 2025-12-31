import winston from "winston";

const isTest = process.env.NODE_ENV === "test";
const level = process.env.NODE_ENV !== "production" ? "verbose" : "info";

const logger = winston.createLogger({
  silent: isTest,
  level,
  format: winston.format.json(),
  transports: [
    // improve this config later
    new winston.transports.Console({
      format: winston.format.simple(),
    }),
  ],
});

export default logger;
