# framework-codegen-typescript

TypeScript ACTR protoc plugin for Actor-RTC client stubs.

This plugin generates outbound RPC client helpers only. Generate protobuf
message code with `protoc-gen-es`, then run this plugin for remote service
client stubs such as `*_client.ts`.

TypeScript workloads are package-first. Author workload code with
`@actrium/actr-workload` and build components with `actr-workload-ts`. This
plugin does not generate TypeScript local service dispatchers or workload
runtime entrypoints.

## Build

```bash
npm install
npm run build
npm run bundle
```

## Usage

```bash
protoc \
  --plugin=protoc-gen-es=./node_modules/.bin/protoc-gen-es \
  --es_out=generated \
  --es_opt=target=ts \
  remote/echo.proto

protoc \
  --plugin=protoc-gen-actrframework-typescript=./scripts/protoc-gen-actrframework-typescript \
  --actrframework-typescript_out=generated \
  --actrframework-typescript_opt=target=ts,RemoteFiles=remote/echo.proto,RemoteFileMapping=remote/echo.proto=acme:EchoService \
  remote/echo.proto
```
