# actr-web-protoc-codegen

Protoc code generator for creating actr-web code from Protobuf definitions.

## Features

- ✅ Generate Rust WASM actor code from `.proto` files
- ✅ Generate TypeScript type definitions
- ✅ Generate TypeScript ActorRef wrapper classes
- ✅ Optionally generate React Hooks
- 🔄 Automated code formatting
- 🔄 Custom template support

## Usage

### Option 1: use it from `build.rs` (recommended)

```rust
// build.rs
fn main() {
    use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

    let config = WebCodegenConfig::builder()
        .proto_file("proto/echo.proto")
        .rust_output("src/generated")
        .ts_output("../packages/web-sdk/src/generated")
        .with_react_hooks(true)
        .include("proto")
        .build()
        .expect("Invalid config");

    WebCodegen::new(config)
        .generate()
        .expect("Failed to generate code");

    println!("cargo:rerun-if-changed=proto");
}
```

### Option 2: use it through `actr-cli`

```bash
# Install `actr-cli` with web support
cargo install actr-cli --features web

# Generate code
actr gen --platform web \
  --input proto/ \
  --output crates/actors/src/generated/ \
  --ts-output packages/web-sdk/src/generated/ \
  --react-hooks
```

### Option 3: use the programmatic API

```rust
use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

let config = WebCodegenConfig {
    proto_files: vec!["proto/echo.proto".into()],
    rust_output_dir: "src/generated".into(),
    ts_output_dir: "../web-sdk/src/generated".into(),
    generate_react_hooks: true,
    includes: vec!["proto".into()],
    format_code: true,
    custom_templates_dir: None,
};

let codegen = WebCodegen::new(config);
let files = codegen.generate()?;

// Write files
files.write_to_disk()?;
```

## Generated Layout

### Rust side (WASM)

```
src/generated/
├── mod.rs
├── echo.rs          # EchoActor
└── ...
```

### TypeScript side

```
src/generated/
├── index.ts
├── echo.types.ts         # type definitions
├── echo.actor-ref.ts     # EchoActorRef class
├── use-echo.ts           # optional useEcho hook
└── ...
```

## Configuration Options

| Option | Type | Required | Description |
|------|------|------|------|
| `proto_files` | `Vec<PathBuf>` | ✅ | List of proto files |
| `rust_output_dir` | `PathBuf` | ✅ | Rust output directory |
| `ts_output_dir` | `PathBuf` | ✅ | TypeScript output directory |
| `generate_react_hooks` | `bool` | ❌ | Generate React Hooks, default `false` |
| `includes` | `Vec<PathBuf>` | ❌ | Proto include paths |
| `format_code` | `bool` | ❌ | Format generated code, default `false` |
| `custom_templates_dir` | `Option<PathBuf>` | ❌ | Custom template directory |

## Status

- [x] Base architecture and configuration
- [x] Full proto parsing with a handwritten parser
- [x] Rust actor method generation
- [x] TypeScript type generation
- [x] TypeScript ActorRef method generation
- [x] React Hooks generation
- [x] Streaming method support
- [x] Formatting integration with `rustfmt` and `prettier`/`dprint`
- [x] Unit tests
- [ ] Optional `prost-build` integration
- [ ] Custom template support
- [ ] Integration tests
- [ ] Performance tuning

## Examples

See the example projects under `examples/`:

- `examples/echo/` - basic Echo service example
- `examples/simple-rpc/` - end-to-end RPC example

## License

Apache License 2.0
