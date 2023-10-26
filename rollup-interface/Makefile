.PHONY: help

help: ## Display this help message
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

check-no-std: ## Checks that project compiles without std
	# check bare metal
	cargo check --no-default-features --target thumbv6m-none-eabi
	cargo check --no-default-features --target thumbv6m-none-eabi --features native
