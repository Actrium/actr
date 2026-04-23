# protoc-gen-python

Python code generation is now aligned with the repository's `package-first` direction.

## Current Status

- Client-side protobuf helpers remain valid.
- Source-defined Python service workloads were removed.
- If you need to host a service, build a verified `.actr` package and run it with Rust `Hyper.attach(...)`.

## Recommended Flow

1. Generate Python protobuf/client helpers.
2. Start `ActrNode` with `ActrNode.from_toml("manifest.toml")`.
3. Discover the remote actor with `ActrRef.discover(...)`.
4. Call it explicitly with `ActrRef.call(Dest.actor(target), route_key, request)`.
