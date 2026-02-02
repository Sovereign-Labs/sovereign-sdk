---
name: detect-flaky-tests
description: Identify flaky tests by comparing failures across multiple CI runs. Use when the user suspects flaky tests, sees intermittent failures, or wants to analyze test reliability.
---

# Detect Flaky Tests

Identify flaky tests by comparing failures across multiple CI runs.

## Prerequisites

- `gh` CLI installed and authenticated (`gh auth login`)

## Instructions

### Step 1: Get Recent Workflow Runs

Get last N runs from the specified branch (default: dev):
```bash
gh run list --workflow=Rust --branch=dev --limit=10 --json databaseId,conclusion,headSha,createdAt
```

### Step 2: Download Failed Job Logs

For each run, get failed test jobs only (`nextest`, `nextest_all_features`, `coverage`):
```bash
gh run view <run-id> --json jobs --jq '.jobs[] | select((.name == "nextest" or .name == "nextest_all_features" or .name == "coverage") and .conclusion == "failure") | {id: .databaseId, name: .name}'
```

Download logs to `ci-logs/flaky-analysis/run-<run-id>/`:
```bash
gh run view --job <job-id> --log | perl -pe 's/\e\[[0-9;]*m//g' > ci-logs/flaky-analysis/run-<run-id>/<job_name>.log
```

### Step 3: Extract Failed Tests

Parse each log for failed test names. Look for patterns:
- `FAILED` followed by test path
- `test result: FAILED`
- Specific test framework output patterns

### Step 4: Correlate Failures

Build a matrix: test name vs run ID (pass/fail).

Identify:
- **Flaky tests**: Fail in some runs, pass in others
- **Consistently failing**: Fail in all/most runs (real bugs)
- **New failures**: Only fail in recent runs

### Step 5: Report

Provide summary:
- List of flaky tests with failure rate (e.g., "failed 3/10 runs")
- List of consistently failing tests
- Recommendations:
  - Flaky tests to investigate or quarantine
  - Real failures to fix

## Notes

- Run this on dev to detect flakiness independent of PRs
- Consider running on PR branches to check if PR introduced flakiness
