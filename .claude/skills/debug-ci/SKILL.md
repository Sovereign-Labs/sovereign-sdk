---
name: debug-ci
description: Debug failed CI tests by fetching workflow logs from GitHub Actions and analyzing failures. Use when the user mentions CI failures, test failures, or wants to understand why their PR's CI is failing.
---

# Debug CI

Debug failed CI tests by fetching workflow logs and analyzing failures.

## Prerequisites

- `gh` CLI installed and authenticated (`gh auth login`)

## Instructions

### Step 1: Identify Current PR

Get PR info for current branch:
```bash
gh pr view --json number,title --jq '"\(.number) \(.title)"' 2>/dev/null || echo "no-pr"
```

If no PR exists, use branch name for the log directory.

### Step 2: Get Latest Workflow Run

Get latest "Rust" workflow run for current branch:
```bash
BRANCH=$(git branch --show-current)
gh run list --workflow=Rust --branch=$BRANCH --limit=1 --json databaseId,conclusion,headSha,createdAt
```

### Step 3: Get Job Information

List all jobs from the run:
```bash
gh run view <run-id> --json jobs --jq '.jobs[] | {id: .databaseId, name: .name, conclusion: .conclusion}'
```

### Step 4: Download Logs

Create directory and download all job logs, stripping ANSI color codes:
```bash
mkdir -p ci-logs/pr-<PR_NUMBER>/
# or: mkdir -p ci-logs/branch-<BRANCH_NAME>/

gh run view --job <job-id> --log | perl -pe 's/\e\[[0-9;]*m//g' > ci-logs/pr-<PR_NUMBER>/<job_name>_<job_id>.log
```

Sanitize job names for filenames (replace spaces/special chars with underscores).

### Step 5: Analyze Failures

1. Read downloaded log files
2. Search for failure patterns: `FAILED`, `error[E`, `panicked at`, `assertion failed`
3. Extract test names and error messages
4. Read PR diff (`gh pr diff`) to correlate failures with changes
5. Categorize failures:
   - **Real failure**: Error in code touched by PR
   - **Flaky test**: Known flaky or unrelated to changes
   - **Infrastructure**: Network/timeout issues

### Step 6: Report

Provide summary:
- PR number and title
- Run status and commit SHA
- List of downloaded log file paths
- Categorized failures with file:line references
- Recommendations for next steps

## Notes

- The `ci-logs/` directory should be in `.gitignore`
- Focus on relevant log sections; files can be large
