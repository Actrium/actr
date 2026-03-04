# Examples

This folder contains runnable ACTR TypeScript examples.

## Prerequisites

From the repo root:

```bash
cd actr-ts
npm install
npm run build
```

## EchoTwice Server

Run the local EchoTwice server:

```bash
node --import tsx examples/echo-twice-server/index.ts
```

Service details:
- ActrType: `acme+EchoTwiceService`
- Route: `echo_twice.EchoTwiceService.EchoTwice`

## Echo Client

The client sends two RPCs:
- One to `EchoService` (route `echo.EchoService.Echo`)
- One to `EchoTwiceService` (route `echo_twice.EchoTwiceService.EchoTwice`)

Run the client:

```bash
node --import tsx examples/echo-client/index.ts
```

Notes:
- Make sure your target services are running and discoverable.
- The client uses `Actr.toml` in `examples/echo-client` for discovery.
