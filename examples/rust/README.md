# ACTR Examples Workspace

This repository collects ready-to-run ACTR examples and is now organized as a single Rust workspace targeting edition 2024.

For the cross-language audio capture demo, use the dedicated guide at `../audio-capture/README.md`.

## Actrix Configuration

All examples share the workspace-root `actrix-config.toml`. Do not keep or add per-example copies; start scripts launch actrix from the repository root using this config.

## Actr Configuration

Most example binaries expect a local `actr.toml` in their crate directory. The repository only tracks `Actr.example.toml`; the start scripts copy it for you. If you run binaries directly with `cargo run`, create your own config first (one-time per crate):

```bash
cp data-stream/sender/Actr.example.toml data-stream/sender/actr.toml
cp data-stream/receiver/Actr.example.toml data-stream/receiver/actr.toml
cp shell-actr-echo/server/Actr.example.toml shell-actr-echo/server/actr.toml
cp shell-actr-echo/client/Actr.example.toml shell-actr-echo/client/actr.toml
cp package-echo/client/Actr.example.toml package-echo/client/actr.toml
cp package-echo/client-guest/Actr.example.toml package-echo/client-guest/actr.toml
cp media-relay/actr-a/Actr.example.toml media-relay/actr-a/actr.toml
cp media-relay/actr-b/Actr.example.toml media-relay/actr-b/actr.toml
```

`package-echo` is the exception for the packaged server host: it uses tracked runtime-config templates under `package-echo/`. `package-echo/start.sh` regenerates `server-actr.toml`, while `package-echo/start_tmp_echo_actr.sh` regenerates `tmp_server-actr.toml`, before running `actr run`.

## Prerequisites

The code generators depend on these CLI tools (all installable via `cargo install`):

- `protoc-gen-prost` – protobuf to Rust types
- `protoc-gen-actrframework` – Actr framework glue generator
- `actr` CLI (from the `actr-cli` crate)

Install manually if needed (use `--force` to update to the latest on each run):

```bash
cargo install --force protoc-gen-prost actr-framework-protoc-codegen actr-cli
```

The `start.sh` scripts run a preflight check and will auto-install/update these tools to the latest version before running code generation.

## Usage

Run workspace commands from the repository root. Examples:

- `bash data-stream/start.sh` – spin up actrix (root config), receiver, and sender.
- `bash shell-actr-echo/start.sh` – run the echo server/client against the root actrix config.
- `bash package-echo/start.sh` – run the signed package echo host/client demo against the root actrix config.
- `bash package-echo/start_tmp_echo_actr.sh` – generate a temporary `echo-actr-xx` service via `actr init/install/gen` and run the package echo demo against it.
- `bash package-echo/manual-runtime-lifecycle.sh` – verify detached runtime lifecycle commands against the package-backed server config.
- `bash media-relay/start.sh` – launch the relay demo; actrix starts from the workspace root config.
- `../audio-capture/README.md` – run the Swift sender plus Rust receiver audio capture demo.

### Running Individual Examples

You can run individual examples using either `--bin` or `-p`:

```bash
# Using --bin (explicit binary name)
cargo run --bin echo-real-server
cargo run --bin echo-real-client-app
cargo run --bin data-stream-sender
cargo run --bin data-stream-receiver
cargo run --bin actr-a-relay
cargo run --bin actr-b-receiver

# Using -p (package name, equivalent)
cargo run -p echo-real-server
cargo run -p echo-real-client-app
cargo run -p data-stream-sender
cargo run -p data-stream-receiver
cargo run -p actr-a-relay
cargo run -p actr-b-receiver
```

Both `--bin` and `-p` work identically in a workspace context.

## Example Run Guides (three scenarios)

- **Data Stream** – file transfer between `receiver` and `sender`.
  - Start: `bash data-stream/start.sh`
  - Behavior: boots actrix (root config) → receiver example → sender example; verifies chunk reception.
- **Shell Actr Echo** – echo RPC between shell client and workload server.
  - Start: `bash shell-actr-echo/start.sh`
  - Codegen: runs `actr gen --input=proto --output=src/generated` in `shell-actr-echo/server` and `actr gen --input=proto --output=src/generated --no-scaffold` in `shell-actr-echo/client` before launching.
  - Behavior: boots actrix (root config) → server example → client example; asserts echo reply contains server response.
- **Media Relay** – media frames relayed from `actr-a` to `actr-b`.
  - Start: `bash media-relay/start.sh`
  - Behavior: boots actrix (root config) → `actr-b` example → `actr-a` example; checks frame reception; exits automatically on success.
- **Package Echo** – echo RPC against a host that loads a signed `.actr` package built locally.
  - Start: `bash package-echo/start.sh`
  - Behavior: boots actrix (root config) → builds and signs the local `echo-actr` package → regenerates `server-actr.toml` → runs `actr run -c server-actr.toml` → launches the client example; asserts echo reply contains the packaged service response.
  - Temp scaffold variant: `bash package-echo/start_tmp_echo_actr.sh`
  - Temp behavior: creates a temporary `echo-actr-xx` Rust service project via `actr init`, runs `actr install` + `actr gen -l rust`, then reuses `package-echo/start.sh` with that generated workload and the isolated `tmp_server-actr.toml`.
