# Releases

This document describes how to cut a release of the Sovereign SDK

## Pick a version number

- [ ] Decide a new version number. During alpha, This should have the form `vA.B.C-alpha` where `A, B, C` are natural numbers. Since we're pre `1.0` releases which are not backwards compatible should
      get a new `minor` version, while releases which are backwards compatible only receive a `patch` version bump. Since almost all of our releaese contain breaking changes, we'll almost always bump the minor version (i.e. from `v0.1.0-alpha` to `v0.2.0-alpha`)
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
- [ ] Create a new PR (e.g. `release/{version}`). Ensure all tests and lints pass
- [ ] Once the PR is approved, release the crates

## Release

- [ ] Tag the new commit on main with the version
- [ ] Push the tag (`git push origin {version} `)[^1]
- [ ] Release all packages from `packages_to_publish.txt` in order
