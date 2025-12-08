#!/bin/bash

# The script:
# - uses the desired version of the solana cli for building programs
# - resets the solana cli to the desired version after building programs
# - builds the programs, accepting an arg for what types of programs to build
# - if no arg is given, builds all programs

# The first argument is the type of program to build
PROGRAM_TYPE="${1:-all}"

SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS="1.14.20"

# The paths to the programs
CORE_PROGRAM_PATHS=("../program")
TOKEN_PROGRAM_PATHS=()

build_program () {
    PROGRAM_PATH=$1
    log "Building $PROGRAM_PATH"
    pushd $PROGRAM_PATH
    cargo build-sbf
    popd
}

build_programs () {
    # The list of programs to build
    PROGRAM_PATH_LIST=()

    BUILD_ALL="false"

    if [ $PROGRAM_TYPE == "all" ]; then
        BUILD_ALL="true"
    fi

    log "Building programs of type: $PROGRAM_TYPE"

    if [ $PROGRAM_TYPE == "token" ] || [ $BUILD_ALL == "true" ] ; then
        PROGRAM_PATH_LIST+=("${TOKEN_PROGRAM_PATHS[@]}")
    fi

    if [ $PROGRAM_TYPE == "core" ] || [ $BUILD_ALL == "true" ] ; then
        PROGRAM_PATH_LIST+=("${CORE_PROGRAM_PATHS[@]}")
    fi

    log "Building programs: ${PROGRAM_PATH_LIST[@]}"

    # Build the programs
    for PROGRAM_PATH in "${PROGRAM_PATH_LIST[@]}"
    do
        build_program $PROGRAM_PATH
    done
}

get_current_solana_cli_version () {
    # `solana --version` expected output is:
    #    solana-cli 1.18.18 (src:83047136; feat:4215500110, client:SolanaLabs)
    # So we can use awk to get the second field
    echo $(solana --version | awk '{print $2}')
}

set_solana_cli_version () {
    NEW_VERSION=$1

    if [ $NEW_VERSION == $SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS ]; then
        ./install-solana-1.14.20.sh
    else
        sh -c "$(curl -sSfL https://release.anza.xyz/v$NEW_VERSION/install)"
    fi
}

log () {
    echo "#### $@"
}

# Get the current version of the solana cli
SOLANA_CLI_VERSION_AT_START=$(get_current_solana_cli_version)

cleanup () {
    # Only reset if we changed the version in the first place
    if [ "$SOLANA_CLI_VERSION_AT_START" != "$SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS" ] && \
       [ ! -z "$SOLANA_CLI_VERSION_AT_START" ]; then
        log "Resetting Solana CLI version back to $SOLANA_CLI_VERSION_AT_START..."
        set_solana_cli_version $SOLANA_CLI_VERSION_AT_START
    fi
}

main () {
    trap cleanup EXIT

    # If the current version is not the latest version, update the solana cli
    if [ $SOLANA_CLI_VERSION_AT_START != $SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS ] ; then
        log "Temporarily changing Solana CLI version from $SOLANA_CLI_VERSION_AT_START to $SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS..."
        set_solana_cli_version $SOLANA_CLI_VERSION_FOR_BUILDING_PROGRAMS
    fi

    # Build the programs
    build_programs

    cleanup
}

main
