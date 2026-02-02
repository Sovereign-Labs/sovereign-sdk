# Add Changelog Entry

Add a new entry to CHANGELOG.md for the current PR.

## Instructions

1. **Get PR number**: Check the current git branch name for a PR number pattern (e.g., `ross/feature-123` or `fix-456`). If not found, ask the user for the PR number.

If PR does not exist yet add a placeholder like `#PR_NUMBER`.

2. **Description**: Attempt to derive a brief description of the change from the git commits on the current branch. 

If not possible, prompt the user to provide a description.
   - Start with the affected component (e.g., "EVM:", "DB:", "API:")
   - Use concise, clear language
   - For breaking changes, prefix with `**Breaking Change**` or `**Breaking DB Change**` or `**Breaking EVM Change**` as appropriate

3. **Determine today's date**: Use format `YYYY-MM-DD`

4. **Read CHANGELOG.md** and check if today's date header already exists

5. **Add the entry**:
   - Format: `- #PR_NUMBER Description`
   - If today's date section exists, add the entry under it
   - If today's date section doesn't exist, create a new date header at the top of the file (after any title) and add the entry

6. **Show the user** the added entry and confirm it was added successfully

## Example Entry Formats

```markdown
# 2026-01-18
- #2353 EVM: Add support for new opcode.
- #2354 **Breaking Change** Removes deprecated `old_method` from API.
- #2355 **Breaking DB Change** Changes storage format for accounts. Requires state wipe.
```

## Notes

- Entries should be concise but descriptive enough to understand the change
- Always include the PR number with `#` prefix
- Breaking changes must be clearly marked
- The CI checks for `#PR_NUMBER` in CHANGELOG.md for demo-rollup changes
