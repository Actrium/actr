# SPDX-License-Identifier: Apache-2.0
#
# Build an actr workload authored in C as a wasm32-wasip2 Component.
#
# Pipeline:
#   1. wit-bindgen c            — generate canonical-ABI bindings from
#                                 actr-workload.wit for the guest side
#   2. clang (wasi-sdk)         — compile C + generated bindings for
#                                 wasm32-wasip2; the driver invokes
#                                 wasm-component-ld automatically when the
#                                 target is wasip2, producing a proper
#                                 Component Model component
#   3. wasm-tools component wit — verify the produced component exports the
#                                 actr-workload-guest world exactly
#
# Required tools (versions pinned repo-wide — see AGENTS.md / Phase 1 notes):
#   wit-bindgen         >= 0.57.1
#   wasm-component-ld   >= 0.5.22     (must be on $PATH so clang can invoke it)
#   wasm-tools          >= 1.247.0
#   clang + wasi-sdk    >= 24         (provides wasi-libc headers + builtins
#                                      for --target=wasm32-wasip2)
#
# Override WASI_SDK on the command line if installed elsewhere, e.g.
#   make WASI_SDK=/opt/wasi-sdk-24
#
# Alternative: if your system clang already has wasi-libc configured for
# wasip2 (e.g. via a vendored wasi-sysroot), set WASI_CLANG=clang and the
# build will skip --sysroot handling.

WIT_FILE      := ../../../core/framework/wit/actr-workload.wit
WORLD         := actr-workload-guest
GEN_DIR       := gen
OUT           := echo.wasm

WASI_SDK      ?= /opt/wasi-sdk
WASI_CLANG    ?= $(WASI_SDK)/bin/clang
WASI_SYSROOT  ?= $(WASI_SDK)/share/wasi-sysroot

WIT_BINDGEN   ?= wit-bindgen
WASM_TOOLS    ?= wasm-tools

# If WASI_CLANG points at a wasi-sdk clang, it already knows its sysroot;
# otherwise forward one explicitly when building. Users of a generic clang
# can override SYSROOT_FLAG to empty.
ifeq ($(WASI_CLANG),$(WASI_SDK)/bin/clang)
SYSROOT_FLAG :=
else
SYSROOT_FLAG := --sysroot=$(WASI_SYSROOT)
endif

CFLAGS := -O2 -Wall -Wextra --target=wasm32-wasip2 $(SYSROOT_FLAG) -I$(GEN_DIR)

GEN_SRCS := $(GEN_DIR)/actr_workload_guest.c
GEN_HDRS := $(GEN_DIR)/actr_workload_guest.h
GEN_COMP := $(GEN_DIR)/actr_workload_guest_component_type.o

.PHONY: all bindings component verify clean

all: verify

bindings: $(GEN_HDRS)

$(GEN_HDRS) $(GEN_SRCS) $(GEN_COMP): $(WIT_FILE)
	@mkdir -p $(GEN_DIR)
	$(WIT_BINDGEN) c --world $(WORLD) --out-dir $(GEN_DIR) $(WIT_FILE)

$(OUT): src/main.c $(GEN_SRCS) $(GEN_HDRS) $(GEN_COMP)
	$(WASI_CLANG) $(CFLAGS) \
		-o $@ \
		src/main.c $(GEN_SRCS) $(GEN_COMP)

component: $(OUT)

verify: component
	@echo "==> wasm-tools component wit $(OUT)"
	$(WASM_TOOLS) component wit $(OUT)

clean:
	rm -rf $(GEN_DIR) $(OUT)
