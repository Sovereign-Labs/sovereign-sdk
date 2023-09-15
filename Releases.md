# Releases

This document describes how to cut a release of the Sovereign SDK.

<!-- https://github.com/thlorenz/doctoc -->
<!-- doctoc Releases.md --github --notitle --maxlevel 2 -->
<!-- START doctoc generated TOC please keep comment here to allow auto update -->
<!-- DON'T EDIT THIS SECTION, INSTEAD RE-RUN doctoc TO UPDATE -->

- [Releases](#releases)
  - [Before starting](#before-starting)
  - [Pick a version number](#pick-a-version-number)
  - [Make sure the documentation and tutorials are up to date](#make-sure-the-documentation-and-tutorials-are-up-to-date)
  - [Update all crate versions](#update-all-crate-versions)
  - [Prepare the PR](#prepare-the-pr)
  - [Release to `crates.io`](#release-to-cratesio)
  - [Create a tag and a GitHub release](#create-a-tag-and-a-github-release)
  - [Merge to `nightly` and `stable`](#merge-to-nightly-and-stable)

<!-- END doctoc generated TOC please keep comment here to allow auto update -->

## Before starting

Before starting, ensure your local copy of the repository is up to date with the latest commit on `nightly` and has no untracked or uncommitted changes. Making a fresh clone of the repository is a good way to ensure this.

## Pick a version number

- [ ] Decide a new version number. During alpha, This should have the form `vA.B.C-alpha` where `A, B, C` are natural numbers. Since we're pre `1.0` releases which are not backwards compatible should
      get a new `minor` version, while releases which are backwards compatible only receive a `patch` version bump. Since almost all of our releases contain breaking changes, we'll almost always bump the minor version (i.e. from `v0.1.0-alpha` to `v0.2.0-alpha`)
  - [ ] Don't forget the `v`!
- [ ] Create a new local branch named `release/{version}` (e.g. `release/v0.2.0-alpha`) from `nightly`.

## Make sure the documentation and tutorials are up to date
- [ ] Audit the getting-started documentation and ensure there's no breakages:
  - [ ] Manually run the steps from `examples/demo-rollup/README.md`.
  - [ ] Manually run the steps from `examples/demo-prover/README.md`.
- [ ] Review all the other tutorials and ensure explanations are up to date:
  - [ ] `examples/demo-nft-module/README.md`
  - [ ] `examples/demo-simple-stf/README.md`
  - [ ] `examples/demo-stf/README.md`
  - [ ] `examples/demo-nft-module/README.md`
- [ ] Audit `packages_to_publish.yml` and ensure that all relevant _library_ crates are included. Binaries (such as `demo-rollup`) and other internal crates not intended to be used by SDK users (such as `sov-modules-schemas`) should not be included. The list must remain pre-sorted by dependencies, so that crates are published in the correct order.
- [ ] Commit any changes.

## Update all crate versions

For each and every crate in this repository, you'll need to do three things:

1. [ ] Update the crate's version number.
2. [ ] Update the crate's `sov-*` dependencies to the new version, unless they are sourced from a relative `path = "..."` instead of a version number. In that case there's no dependency version to update, and you can skip it.
3. [ ] Update the crate's `sov-*` dev-dependencies to the new version, just like you did in step 2.

The `cargo set-version` subcommand supplied by [`cargo-edit`](https://github.com/killercup/cargo-edit) does all of this for you automatically. Invoke it like this:

```
$ cargo set-version 0.2.0  # From the root of the repository
```

Note that `cargo set-version` only acts **within** the current workspace, so you'll have to **run it once for every workspace** in the repository. You can find all workspaces by searching for `[workspace]`.

Dependencies across workspace boundaries will not be updated automatically by `cargo set-version`. After running the command, do a final search for the old version number and ensure that it's not used anywhere (except possibly by unrelated 3rd-party dependencies). The good news is that even if you miss any dependency during this step, it will not block the release because only crates within the root workspaces are published to <https://crates.io>.

## Prepare the PR

At this point, the branch you're working on should contain two kinds of changes:

1. (Optional.) Documentation updates, tutorial fixes, etc..
2. Updated Cargo manifests.
3. Updated `Cargo.lock` files.

After committing these changes, you should open a new PR **against `nightly`**. Ensure all tests and lints pass but **do not merge just yet!**

## Release to `crates.io`

**After** the PR is approved but **before** it's merged, you should release the crates in `packages_to_publish.yml` to <https://crates.io>. Go through the list of crates to release _in order_ and run these commands:

- [ ] `$ cargo publish --dry-run -p {crate}` to ensure there are no issues.
- [ ] `$ cargo publish -p {crate}` to actually release the crate.

## Create a tag and a GitHub release

- [ ] Create a new tag named `{version}` (e.g. `v0.2.0-alpha`) from the `release/{version}` branch.
- [ ] Push the tag (`git push origin {version}`)
- [ ] Create a new GitHub release from the tag [here](https://github.com/Sovereign-Labs/sovereign-sdk/releases/new). The release title should be the version number (e.g. `v0.2.0-alpha`). Consult the team to decide on a good release description.

## Merge to `nightly` and `stable`

- [ ] Merge your PR to `nightly`.
- [ ] Merge `nightly` to `stable` with a new PR (example [here](https://github.com/Sovereign-Labs/sovereign-sdk/pull/866#pullrequestreview-1627315759)). Make sure no commits that are not part of the release get merged accidentally into `stable`.
