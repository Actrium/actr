module echo-workload

go 1.23

// wit-bindgen-go emits cm.* helper types (Result, List, Option, Variant
// codecs). Pin the Bytecode Alliance runtime that wit-bindgen-go 0.6.x
// generates against. Replace with `go get` once the build.sh has been run
// at least once on the host (which also runs `go mod tidy`).
require go.bytecodealliance.org/cm v0.3.0
