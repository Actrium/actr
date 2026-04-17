# actr CLI

Command-line tool for Actor-RTC projects. Handles project scaffolding, codegen,
packaging (`.actr`), signing, installation of service dependencies, and runtime
instance management.

## Quick reference (19 top-level subcommands)

```
Development (flat, 5):  actr init | gen | build | check | doc
Runtime (flat, 7):      actr run | ps | logs | start | stop | restart | rm
Resources (grouped, 4): actr deps | pkg | registry | dlq
Meta (3):               actr config | version | completion
```

Full help: `actr --help`, then drill down with `actr <command> --help`.

---

## Organization principles

These rules govern where a new command lives. Add commands that fit; if a new
command doesn't fit, update this document **before** adding code.

### 1. Audience determines grouping

Commands serve three audiences. Keep their paths short in proportion to how
often the audience types them.

| Audience | Typical commands | Path depth |
|---|---|---|
| **Developer** (writes actor code) | `init`, `gen`, `build`, `check`, `doc` | flat |
| **Operator** (runs workloads) | `run`, `ps`, `logs`, `start`, `stop`, `restart`, `rm` | flat |
| **Integrator / publisher** (manages deps, packages, registry) | `deps ...`, `pkg ...`, `registry ...`, `dlq ...` | grouped |

**High-frequency stays flat** (Docker/Cargo precedent). **Low-frequency or
fine-grained stays grouped** so the top level stays scannable.

### 2. Group by resource, not by phase

Groups are named after **what the subcommand operates on**:

- `deps` — local dependency manifest / lockfile
- `pkg` — a local `.actr` package file (sign, verify, keygen)
- `registry` — the remote registry / AIS (discover, publish, fingerprint)
- `dlq` — the Dead Letter Queue

**Do not** group by workflow phase (`build`, `release`, `ops`, etc.) — those
names rot as the workflow evolves.

### 3. Full-flow commands vs fine-grained operations

- `actr build` — full source-to-package pipeline (compile → hash → sign → zip).
  The default entry point for developers.
- `actr pkg sign / verify / keygen` — single-step operations against an existing
  manifest or package. Intended for debugging, CI glue, or signed-build splits.

The rule: **one top-level command per full workflow**; put individual steps in
a group. Never expose both `foo` and `group foo` with overlapping semantics.

### 4. Parameter conventions

- Manifest path: `--manifest-path` / `-m` (follows Cargo). Default
  `manifest.toml`.
- Runtime config (`actr.toml`): `--config` / `-c`.
- Output file: `--output` / `-o`.
- Signing key: `--key` / `-k`.
- Never reuse `-f`; reserve single-letter short flags for obvious mappings.

### 5. Internal flags stay hidden

Flags that only another subcommand or a spawned child should set MUST use
`#[arg(long = "internal-...", hide = true)]`. Users browsing `--help` shouldn't
see implementation plumbing.

### 6. Every subcommand implements `core::Command`

All command types implement [`actr_cli::core::Command`][trait]:

```rust
async fn execute(&self, ctx: &CommandContext) -> anyhow::Result<CommandResult>;
fn required_components(&self) -> Vec<ComponentType>;
fn name(&self) -> &str;
fn description(&self) -> &str;
```

Dispatch is a single call in `src/cli.rs::run`; the service container is only
built when `required_components()` is non-empty. This keeps commands that don't
need the network (e.g. `keygen`, `version`, `pkg sign`) cheap.

[trait]: ./src/core/container.rs

### 7. Output style

Return a structured `CommandResult` (variants: `Success`, `Install`,
`Validation`, `Generation`, `Error`). The dispatcher in `cli.rs` renders it.
Avoid `println!` inside command logic when the result should flow through the
standard renderer.

### 8. No backwards-compat aliases (pre-1.0)

Until the CLI is released, break freely. Renames go in a single PR with
everything — call sites, e2e tests, scaffold templates, docs — updated atomically.
Do **not** ship transitional aliases.

---

## Adding a new command — checklist

1. Decide the audience (Dev / Operator / Integrator) → determines whether it
   sits at the top level or inside a group.
2. Pick a resource-oriented noun if grouping is required; extend an existing
   group before creating a new one.
3. Add the `#[derive(Args)]` struct in `src/commands/<name>.rs` and implement
   `core::Command`.
4. Register the variant in `src/cli.rs::Commands` and `as_command()`.
5. Re-export the type from `src/commands/mod.rs`.
6. Add a smoke assertion in `cli/tests/cli_shape.rs`.
7. Document the command in this README if it changes the top-level shape.

---

## Runtime config vs manifest

Two TOML files, easily confused:

| File | Owned by | Purpose |
|---|---|---|
| `manifest.toml` | developer | Project metadata, service `[package]`, proto exports, dependencies. Signed as part of `.actr`. |
| `actr.toml` | operator | Runtime config: package path, signaling URL, realm, data dir. Consumed by `actr run`. |

Commands that touch `manifest.toml` use `--manifest-path` / `-m`.
Commands that touch `actr.toml` use `--config` / `-c`.

---

## See also

- `src/cli.rs` — top-level `Cli` struct and dispatcher.
- `src/core/container.rs` — `Command` trait and `ServiceContainer`.
- `src/commands/` — one file per subcommand.
