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

1. Log in into [Namepspace](https://namespace.so/) Web UI
2. Got to "Profiles" section
3. Click "New Profile"
4. Enter the new tag you wish to use (more on tags below) and enter the following values:
    - OS: linux on amd64 
    - Base Image: Custom Ubuntu 24.04
    - Ubuntu-based Custom Image: select "Custom Dockerfile" and paste new Dockerfile you've just updated and tested before.
    - Caching: enable, set the desired size and enable "container images", "git checkouts" and "toolchain download".
      For advanced section of caching set `nightly` as protected branch
5. Click "Update Profile". The icon "Building" next to "Ubuntu-based custom image" will appear. Wait  till it becomes ready.

Now this profile is ready to be used.

More information at the [configure your runners](https://namespace.so/docs/solutions/github-actions#configure-your-runners) section.

### Tags and cache with pre-built images

Custom-based images profile does not support a combination of cache volume tags. 
This means that if the same container profile needs different caches, it needs to be a different profile.

Some details from Namespace:

Custom-based images profile enables container image caching.
This means that Namepsace keeping pulled and unpacked images in the cache.
This caching includes also your new custom base image.
The fact that the custom base image also lives in the cache is a performance optimization today so that subsequent runs do not need to pull it.
But it also means that the image takes space from here.
It also implies that while the runner is running, and files created in your run will allocate space from the cache while the run is ongoing.

## Updating the Workflow file

After profile is ready, it is possible to copy `runs-on` value needed

If container needs io_uring, for instance for NOMT or tests, append `;container.privileged=true;container.host-pid-namespace=true` to the runs on label from namespace

