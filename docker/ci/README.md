# Namespace CI Docker Base Image

This folder contains the [Dockerfile](./Dockerfile) used for [Custom Base Image for Namespace CI](https://namespace.so/docs/solutions/github-actions/custom-base-images).

Before starting, [install the Namespace CLI tool `nsc`](https://namespace.so/docs/reference/cli/installation).

The base image runs as the runner user by default (to match GitHub's runners), which requires using sudo for commands like apt-get.
To iterate on the build faster, you can trigger the build using the nsc CLI instead. 

This will give you the output right in your terminal from this folder:
```
nsc github build-base-image --os-label ubuntu-24.04 --platform linux/amd64 -f Dockerfile
```

The `nsc` tool will run the build and give you the output in case of an error.
This allows for debugging the build but will not push the final image.
Once you have a successful build, you will have to copy the Dockerfile back into the web UI and `profiles` section.
A final build will then be kicked off once you save the profile. You can see the status of this on the profile page:
When you go to the profile page, to the top right of the Dockerfile box should be a status badge saying "building" or "Done".

More information at the [configure your runners](https://namespace.so/docs/solutions/github-actions#configure-your-runners) section.