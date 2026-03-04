# actr-framework-protoc-codegen

Protoc plugin for generating actr-framework code from protobuf service definitions.

## Status

🚧 **Placeholder** - Code will be moved from `cli/crates/protoc-gen-actrframework`

## What it generates

From a protobuf service definition:

```protobuf
service EchoService {
  rpc Echo (EchoRequest) returns (EchoResponse);
}
```

Generates:

1. **Handler trait** - User implements business logic
2. **MessageDispatcher** - Routes messages to handler methods
3. **Workload wrapper** - Integrates with ActrSystem
4. **Message trait impl** - Enables Context::call() type inference

## Architecture

- Uses **MessageDispatcher** (not MessageRouter)
- Uses **Workload::Dispatcher** (not Workload::Router)
- Generates clean, idiomatic Rust code

## Usage

```bash
# Install
cargo install actr-framework-protoc-codegen

# Generate code
protoc --actrframework_out=src/generated proto/*.proto
```

## TODO

- [ ] Move code from cli/crates/protoc-gen-actrframework
- [ ] Update templates to use MessageDispatcher
- [ ] Update templates to use Workload::Dispatcher
- [ ] Add integration tests
- [ ] Add template documentation
