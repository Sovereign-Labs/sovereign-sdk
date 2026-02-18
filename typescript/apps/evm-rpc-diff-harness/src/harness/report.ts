import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import type { CheckResult, CompareReport, ReportSummary } from "./types";

function summarize(checks: CheckResult[]): ReportSummary {
  let pass = 0;
  let fail = 0;
  let notSupported = 0;

  for (const check of checks) {
    if (check.outcome === "PASS") {
      pass += 1;
    } else if (check.outcome === "FAIL") {
      fail += 1;
    } else {
      notSupported += 1;
    }
  }

  return {
    total: checks.length,
    pass,
    fail,
    notSupported
  };
}

function markdownTable(checks: CheckResult[]): string {
  const header = "| Check | Library | Outcome | RPC Methods |";
  const divider = "|---|---|---|---|";
  const rows = checks.map(
    (check) => `| ${check.name} | ${check.library} | ${check.outcome} | ${check.rpcMethods.join(", ")} |`
  );
  return [header, divider, ...rows].join("\n");
}

function stringify(value: unknown): string {
  if (value === undefined) {
    return "undefined";
  }

  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function renderRequests(check: CheckResult): string {
  return [
    "#### Requests (Anvil)",
    "",
    "```json",
    stringify(check.requests.anvil),
    "```",
    "",
    "#### Requests (Rollup)",
    "",
    "```json",
    stringify(check.requests.rollup),
    "```"
  ].join("\n");
}

function renderFailures(checks: CheckResult[]): string {
  const failures = checks.filter((check) => check.outcome === "FAIL");
  if (failures.length === 0) {
    return "No FAIL outcomes detected.";
  }

  return failures
    .slice(0, 10)
    .map((check) => {
      const blocks: string[] = [
        `### ${check.name}`,
        "",
        `- Library: ${check.library}`,
        `- RPC methods: ${check.rpcMethods.join(", ")}`,
        "",
        "#### Diff",
        "",
        "```json",
        stringify(check.diff ?? "No diff captured."),
        "```",
        "",
        "#### Expected (Anvil)",
        "",
        "```json",
        stringify({
          normalized: check.anvil.normalized,
          error: check.anvil.error,
          raw: check.anvil.raw
        }),
        "```",
        "",
        "#### Actual (Rollup)",
        "",
        "```json",
        stringify({
          normalized: check.rollup.normalized,
          error: check.rollup.error,
          raw: check.rollup.raw
        }),
        "```",
        "",
        renderRequests(check)
      ];

      return blocks.join("\n");
    })
    .join("\n\n");
}

function renderNotSupported(checks: CheckResult[]): string {
  const unsupported = checks.filter((check) => check.outcome === "NOT_SUPPORTED");
  if (unsupported.length === 0) {
    return "No NOT_SUPPORTED outcomes detected.";
  }

  return unsupported
    .map((check) => {
      return [
        `### ${check.name}`,
        "",
        "```json",
        stringify({
          rollupError: check.rollup.error,
          rollupRaw: check.rollup.raw,
          diff: check.diff
        }),
        "```"
      ].join("\n");
    })
    .join("\n\n");
}

function toMarkdown(report: CompareReport): string {
  return [
    "# EVM RPC Differential Report",
    "",
    `Generated at: ${report.generatedAt}`,
    "",
    "## Endpoints",
    "",
    `- Anvil RPC: ${report.anvil.rpcUrl}`,
    `- Anvil chainId: ${report.anvil.chainId}`,
    `- Anvil clientVersion: ${report.anvil.clientVersion ?? "unknown"}`,
    `- Rollup RPC: ${report.rollup.rpcUrl}`,
    `- Rollup chainId: ${report.rollup.chainId}`,
    `- Rollup clientVersion: ${report.rollup.clientVersion ?? "unknown"}`,
    "",
    "## Summary",
    "",
    `- Total checks: ${report.summary.total}`,
    `- PASS: ${report.summary.pass}`,
    `- FAIL: ${report.summary.fail}`,
    `- NOT_SUPPORTED: ${report.summary.notSupported}`,
    "",
    "## Outcome Table",
    "",
    markdownTable(report.checks),
    "",
    "## Top Mismatches",
    "",
    renderFailures(report.checks),
    "",
    "## Not Supported",
    "",
    renderNotSupported(report.checks),
    ""
  ].join("\n");
}

export async function writeReport(
  appRoot: string,
  checkResults: CheckResult[],
  anvilMeta: { rpcUrl: string; chainId: bigint; clientVersion?: string },
  rollupMeta: { rpcUrl: string; chainId: bigint; clientVersion?: string }
): Promise<CompareReport> {
  const summary = summarize(checkResults);

  const report: CompareReport = {
    generatedAt: new Date().toISOString(),
    anvil: {
      rpcUrl: anvilMeta.rpcUrl,
      chainId: anvilMeta.chainId.toString(),
      clientVersion: anvilMeta.clientVersion
    },
    rollup: {
      rpcUrl: rollupMeta.rpcUrl,
      chainId: rollupMeta.chainId.toString(),
      clientVersion: rollupMeta.clientVersion
    },
    summary,
    checks: checkResults
  };

  const outputDir = path.join(appRoot, "artifacts");
  await mkdir(outputDir, { recursive: true });

  await writeFile(path.join(outputDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`, "utf-8");
  await writeFile(path.join(outputDir, "report.md"), toMarkdown(report), "utf-8");

  return report;
}
