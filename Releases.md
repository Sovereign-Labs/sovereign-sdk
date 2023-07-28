# Releases

This document describes how to cut a release of the Sovereign SDK

## Pick a version number

- [ ] Decide a new version number. During alpha, This should have the form `vA.B.C-alpha` where `A, B, C` are natural numbers. Since we're pre `1.0` releases which are not backwards compatible should
      get a new `minor` version, while releases which are backwards compatible only receive a `patch` version bump. Since almost all of our releases contain breaking changes, we'll almost always bump the minor version (i.e. from `v0.1.0-alpha` to `v0.2.0-alpha`)
  - [ ] Don't forget the `v`!

## Check Consistency

- [ ] Audit the getting-started documentation
  - [ ] Manually run the steps from `demo-rollup/README.md` to ensure there is no breakage
- [ ] Review all tutorials and ensure explanations are up to date
  - [ ] demo-nft-module/README.md
  - [ ] demo-simple-stf/README.md
  - [ ] demo-stf/README.md
  - [ ] demo-nft-module/README.md
- [ ] Audit `packages_to_publish.txt` and ensure that all relevant _library_ packages are included. Binaries (such as demo-rollup) should not be included.
- [ ] Commit any changes.
- [ ] Change the `package.version` field of all workspace crates from `workspace = true` to the _old_ version number
- [ ] Commit any changes.
- [ ] Bump the workspace `package.version` (this is safe, since now no crates depend on it)
- [ ] Create a new PR (e.g. `release/{version}`) against nightly. Ensure all tests and lints pass but _DO NOT MERGE_
- [ ] Wait for the PR to be approved before merging

## Cut the Release

- [ ] Release all packages from `packages_to_publish.txt` _in order_ (it's pre-sorted by dependencies)
  - [ ] For each crate: change its `package.version` back to `workspace = true`. Bump the versions of all of its `sovereign-sdk` dependencies.
  - [ ] run `cargo publish --dry-run {crate}` to ensure there are no issues
  - [ ] run `cargo publish {crate}` to release the crate
- [ ] Bump the versions and dependency versions of all unreleased crates
- [ ] Commit all changes and push. Be sure to tag your commit with the version.
- [ ] Once CI passes, merge your PR to nightly.
- [ ] Merge from nightly to stable
- [ ] Push the tag (`git push origin {version} `)[^1]
