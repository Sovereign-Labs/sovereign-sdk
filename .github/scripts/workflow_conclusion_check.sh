#!/bin/bash

RETRIES=10
WAIT_TIME=300
REPO="sov-rollup-starter"


while :
do 
    echo " "
    if [[ RETRIES -lt 1 ]]
    then
        echo "Timeout: unable to read workflow_conclusion for ${REPO}"
        exit 1
    fi

    workflow_result="$(
        curl \
        -s \
        -L \
        -H "Accept: application/vnd.github+json" \
        -H "Authorization: token $CI_TRIGGER_ACCESS_TOKEN" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "https://api.github.com/repos/Sovereign-Labs/$REPO/actions/workflows/rust.yml/runs"
    )"

    workflow_status=$(echo $workflow_result | jq -r '.workflow_runs[0].status')
    echo "workflow_status for the ${REPO}: ${workflow_status}"

    workflow_conclusion=$(echo $workflow_result | jq -r '.workflow_runs[0].conclusion')
    echo "workflow_conclusion for the ${REPO}: ${workflow_conclusion}"

    if [[ $workflow_conclusion == "success" ]]
    then
        echo "CI in ${REPO} succeeded."
        break
    fi

    if [[ $workflow_conclusion == "failure" ]]
    then
        echo "CI in ${REPO} failed."
        exit 1
    fi
   
    sleep $WAIT_TIME
    ((RETRIES--))

    echo "Waiting for workflow_conclusion of the ${REPO}. Remainaing RETRIES: ${RETRIES}"
done 