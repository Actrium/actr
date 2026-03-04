BIN := protoc-gen-actrframework-swift
BUILD_DIR := .build/arm64-apple-macosx/release
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin

.PHONY: build install

build:
	swift build -c release --product $(BIN) --arch arm64

install: build
	install -m 0755 $(BUILD_DIR)/$(BIN) $(BINDIR)/$(BIN)
