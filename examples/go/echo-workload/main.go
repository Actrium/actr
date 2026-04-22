// SPDX-License-Identifier: Apache-2.0
//
// Echo workload (Go / TinyGo) — actr Component Model guest.
//
// Implements the `actr-workload-guest` world defined in
// `core/framework/wit/actr-workload.wit`. Built for `wasm32-wasip2` via
// TinyGo + wasm-component-ld, the resulting binary is a Component that
// the actr host loads through `Component::from_binary`.
//
// Minimal hook coverage:
//
//   - dispatch:  echo back the inbound payload prefixed with "echo: "
//   - on-start:  no-op (returns Ok(()))
//
// All other 14 lifecycle / signaling / transport / credential / mailbox
// hooks are left at their wit-bindgen-go default (nil), which the Go
// runtime serialises as "no export" — the host treats absent observation
// hooks as no-ops.
//
// Generated bindings live under `gen/`. They are produced by
// `wit-bindgen-go generate --world actr-workload-guest --out gen
// ../../../core/framework/wit/actr-workload.wit` (see build.sh). The
// import path below assumes the `go.mod` module name `echo-workload`.

package main

import (
	"echo-workload/gen/actr/workload/types"
	workload "echo-workload/gen/actr/workload/workload"

	"go.bytecodealliance.org/cm"
)

func init() {
	// Wire the two hooks we implement; everything else stays nil so the
	// generator emits the export as a stub returning the WIT-default
	// "infallible no-op" / "ok empty result".
	workload.Exports.OnStart = onStart
	workload.Exports.Dispatch = dispatch
}

// main is required by the Go toolchain but is never executed by the
// Component Model host: the WIT exports are invoked directly.
func main() {}

// onStart corresponds to `workload.on-start: func() -> result<_, actr-error>`.
// Returning OK signals the host that the workload is ready to receive
// dispatch calls. We have no startup state to initialise.
func onStart() (result cm.Result[types.ActrErrorShape, struct{}, types.ActrError]) {
	// cm.Result with empty OK payload — the canonical "ok(())" encoding.
	return
}

// dispatch corresponds to `workload.dispatch: func(envelope: rpc-envelope)
// -> result<list<u8>, actr-error>`.
//
// We do not decode the payload as protobuf here (cf. the Rust echo example,
// which goes through protoc-gen-actrframework + prost). The contract for
// this minimal Go example is strictly byte-level: prepend "echo: " and
// return. A real Go workload would import `google.golang.org/protobuf` (or
// a TinyGo-compatible alternative) and round-trip a typed message.
func dispatch(envelope types.RpcEnvelope) (result cm.Result[cm.List[uint8], cm.List[uint8], types.ActrError]) {
	prefix := []byte("echo: ")
	payload := envelope.Payload.Slice()

	out := make([]byte, 0, len(prefix)+len(payload))
	out = append(out, prefix...)
	out = append(out, payload...)

	result.SetOK(cm.ToList(out))
	return
}
