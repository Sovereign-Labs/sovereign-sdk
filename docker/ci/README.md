# Namespace CI Docker Base Image

Using a pre-built image allows skipping the setup of the necessary environment for each job. 
This saves time and increases the stability of CI jobs, as there are fewer opportunities for failure, which has been observed with the installation of ZKVMs

This folder contains the [Dockerfile](./Dockerfile) used for [Custom Base Image for Namespace CI](https://namespace.so/docs/solutions/github-actions/custom-base-images).

## When to update base image

The base image needs to be updated any time the environment changes: Rust toolchain upgrade, ZKVM version upgrade, new tool added to the job

## High-level steps

1. Update and verify [`Dockerfile`](./Dockerfile)
2. Create/update profiles on Namespace
3. Update workflow file

## Updating and verifying `Dockerfile`

Before starting, [install the Namespace CLI tool `nsc`](https://namespace.so/docs/reference/cli/installation).

The contents of the Dockerfile are pasted as plaintext into the Namespace web UI, so it should be self-contained. 
No `COPY` or `ADD` commands should be used. 
This results in duplication with the Makefile, but it is acceptable.


The base image runs as the runner user by default (to match GitHub's runners), which requires using sudo for commands like apt-get.
To iterate on the build faster, you can trigger the build using the `nsc` CLI instead. 

This will give you the output right in your terminal from this folder:
```
nsc github build-base-image --os-label ubuntu-24.04 --platform linux/amd64 -f Dockerfile
```

The `nsc` tool will run the build and give you the output in case of an error.
This allows for debugging the build but will not push the final image.
Once you have a successful build, continue to the next phase.

## How to build a new Namespace profile

1. Log in to the [Namespace](https://namespace.so/) web UI
2. Go to the "Profiles" section
3. Click "New Profile"
4. Enter the new tag you wish to use (more on tags below) and enter the following values:
    - OS: linux on amd64 
    - Base Image: Custom Ubuntu 24.04
    - Ubuntu-based Custom Image: select "Custom Dockerfile" and paste the new Dockerfile you've just updated and tested
    - Caching: enable, set the desired size, and enable "container images", "git checkouts", and "toolchain downloads".
      For the advanced section of caching, set `nightly` as a protected branch
5. Click "Update Profile". The "Building" icon next to "Ubuntu-based custom image" will appear. Wait until it becomes ready.

Now this profile is ready to be used.

More information at the [configure your runners](https://namespace.so/docs/solutions/github-actions#configure-your-runners) section.

### Tags and cache with pre-built images

Custom-based image profiles do not support a combination of cache volume tags. 
This means that if the same container profile needs different caches, it needs to be a different profile.

Some details from Namespace:

Custom-based image profiles enable container image caching.
This means that Namespace keeps pulled and unpacked images in the cache.
This caching includes also your new custom base image.
The fact that the custom base image also lives in the cache is a performance optimization today so that subsequent runs do not need to pull it.
However, it also means that the image takes space from the cache.
Additionally, while the runner is running, any files created during your run will allocate space from the cache.

## Updating the Workflow file

After the profile is ready, you can copy the `runs-on` value needed

If the container needs io_uring (for instance, for NOMT or tests), append `;container.privileged=true;container.host-pid-namespace=true` to the runs-on label from Namespace

