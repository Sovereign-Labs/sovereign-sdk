# Namespace CI Docker Base Image

Using pre-built image allows skipping setting up the necessary environment for each job. 
This allows saving time and increase stability of a CI job, as there's a less opportunity for a failure, which has been observed with installation of ZKVMs

This folder contains the [Dockerfile](./Dockerfile) used for [Custom Base Image for Namespace CI](https://namespace.so/docs/solutions/github-actions/custom-base-images).

## When to update base image

Base image needs to be updated any time the environment changes: rust toolchain upgrade, ZKVM version upgrade, new tool added to the job

## High-level steps

1. Update and verify [`Dockerfile`](./Dockerfile)
2. Create/update profiles on Namespace
3. Update workflow file

## Updating and verify [`Dockerfile`]

Before starting, [install the Namespace CLI tool `nsc`](https://namespace.so/docs/reference/cli/installation).

The contents of Dockerfile are pasted as plaintext into Namespace WEB ui, so it should be self-contained. 
No `COPY` or `ADD` commands. 
This results in duplication between Makefile, but it is acceptable.


The base image runs as the runner user by default (to match GitHub's runners), which requires using sudo for commands like apt-get.
To iterate on the build faster, you can trigger the build using the `nsc` CLI instead. 

This will give you the output right in your terminal from this folder:
```
nsc github build-base-image --os-label ubuntu-24.04 --platform linux/amd64 -f Dockerfile
```

The `nsc` tool will run the build and give you the output in case of an error.
This allows for debugging the build but will not push the final image.
Once you have a successful build, continue to the next phase.

## How to build a new Namespace profile.

1. Log in into namepspace 
2. Got to "Profiles" section
3. Click new profile
4. Enter new tag you wish to use (more on tags below) and enter the following values:
    - OS: linux on amd64 
    - Base Image: Custom Ubuntu 24.04
    - Ubuntu-based Custom Image: select "Custom Dockerfile" and paste new Dockerfile you've just updated and tested before.
    - Caching: enable, set the desired size and enable "container images", "git checkouts" and "toolchain download".
      For advanaced section of caching set `nightly` as protected branch
5. Click "Update Profile". The icon "Building" next to "Ubuntu-based custom image" will appear. Wait  till it becomes ready.

Now this profile is ready to be used.

More information at the [configure your runners](https://namespace.so/docs/solutions/github-actions#configure-your-runners) section.

### Tags and cache with pre-built images

TBD

sov-ubuntu-24.04-amd64-16x32-test-250gb-1-88

## Updating workflow file

After profile is ready, it is possible to copy `runs-on` value needed

TBD: Note on features for I/O uring

