---
name: analyze-ci-speed
description: Analyze compilation time and test durations from CI logs. Use when the user asks about slow builds, slow tests, or wants to optimize CI time.
---

# Analyze CI Speed

Analyze compilation time and test durations from CI logs to identify optimization opportunities.

## Prerequisites

- `gh` CLI installed and authenticated (`gh auth login`)

## Instructions

### Step 1: Get Recent Workflow Runs

Get latest successful run from dev branch:
```bash
gh run list --workflow=Rust --branch=dev --limit=1 --status=success --json databaseId,conclusion,headSha,createdAt
```

### Step 2: Download Test Job Logs

Get test jobs (`nextest`, `nextest_all_features`, `coverage`):
```bash
gh run view <run-id> --json jobs --jq '.jobs[] | select(.name == "nextest" or .name == "nextest_all_features" or .name == "coverage") | {id: .databaseId, name: .name}'
```

Download logs, stripping ANSI codes:
```bash
mkdir -p ci-logs/speed-analysis/
gh run view --job <job-id> --log | perl -pe 's/\e\[[0-9;]*m//g' > ci-logs/speed-analysis/<job_name>.log
```

### Step 3: Extract Compilation Time

Search for cargo build summary in logs:
```
Finished ... in Xm Ys
```

Also look for incremental compilation info and cache hit rates.

### Step 4: Extract Test Durations

Parse nextest output for test durations. Look for patterns like:
```
PASS [   1.234s] crate_name::module::test_name
SLOW [  15.678s] crate_name::module::slow_test
```

Extract all test durations and sort to find top 10 slowest tests.

### Step 5: Analyze and Report

Provide summary:
- Total compilation time
- Top 10 slowest tests with durations
- Any tests marked as SLOW by nextest
- Recommendations:
  - Tests that could be optimized or parallelized
  - Crates with long compilation times
  - Potential for better caching

## Notes

- Run on successful builds to get complete timing data
- Compare across runs to identify regressions
