# Package Runtime Echo E2E

This scenario validates the local package runtime flow end-to-end:

1. Start a local `actrix` node from the scenario config.
2. Use the Admin API to create a dedicated realm and capture its `realm_secret`.
3. Use the Admin API to create and approve the `actrium` manufacturer used by both packages.
4. Generate a temporary Rust echo service with `actr init/install/gen`.
5. Build and publish the signed server package.
6. Build a local client guest package and run the client host.
7. Assert the client receives the echoed reply.

## Requirements

- `cargo`
- `git` if `actrix` is not already available locally
- `curl`
- `jq`
- `sqlite3`
- `python3`

`actrix` does not need to be preinstalled. The scenario resolves it in this order:

1. `ACTRIX_BIN`
2. `actrix` from `PATH`
3. Source code from `../actrix`
4. Clone `https://github.com/Actrium/actrix.git` into `../actrix`, then install from `crates/actrixd`

The auto-install target is the default cargo user bin directory:

- `$CARGO_HOME/bin`
- `~/.cargo/bin` when `CARGO_HOME` is unset

## Run

```bash
bash e2e/package-runtime-echo/run.sh
bash e2e/package-runtime-echo/run.sh "Hello"
```

Useful environment variables:

- `ACTRIX_BIN` overrides the `actrix` executable path.
- `KEEP_TMP=1` keeps the `.tmp/run-*` directory after the run.
- `CLIENT_TIMEOUT_SECONDS` changes the client wait timeout.
- `RUST_LOG` forwards the log level to the host processes.
