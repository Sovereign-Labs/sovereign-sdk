#!/bin/bash

# Node uses package.json in the folder a file resides in to determine how to resolve it
# We need to ensure a package.json with the type of module in it exists for the esm files
# so they're treated as ESM despite not having the `.mjs` file extension in a CJS package
echo '{ "type": "module" }' > ./dist/esm/package.json


# This isn't actually needed since the root package.json is used by include it for good measure
echo '{ "type": "commonjs" }' > dist/node/package.json
