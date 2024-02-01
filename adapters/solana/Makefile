# Variables
TARGET_REPO=git@github.com:rpcpool/yellowstone-grpc.git
TAG=v1.9.0+solana.1.16.15
TMP_DIR := $(shell mktemp -d)
REPO_DIR=$(TMP_DIR)/yellowstone-grpc
GEYSER_DIR=$(REPO_DIR)/yellowstone-grpc-geyser
DEST_DIR := $(PWD)
LIB_NAME=libyellowstone_grpc_geyser.dylib

.PHONY: all clean

all: $(DEST_DIR)/$(LIB_NAME)

$(DEST_DIR)/$(LIB_NAME): $(REPO_DIR)/target/release/$(LIB_NAME)
	cp $< $@

$(REPO_DIR)/target/release/$(LIB_NAME): $(GEYSER_DIR)
	cd $(GEYSER_DIR) && cargo build --release

$(GEYSER_DIR): $(TMP_DIR)/.cloned
	cd $(REPO_DIR) && git checkout $(TAG)
	touch $@

$(TMP_DIR)/.cloned:
	git clone $(TARGET_REPO) $(REPO_DIR)
	touch $@

clean:
	rm -rf $(TMP_DIR)
