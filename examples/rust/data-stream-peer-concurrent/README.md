# Data Stream Peer Concurrent Example

This example demonstrates bidirectional peer-to-peer streaming communication with concurrent support:

```
actr_ref -> local StreamClient handler -> (ctx.call, ctx.send_data_stream) -> remote StreamServer handler -> register_stream
```

## Features

- ✅ **Bidirectional communication**: Both client and server can send DataStream messages
- ✅ **Concurrent support**: Server handles multiple clients concurrently
- ✅ **Peer-to-peer**: Uses actr_ref for actor discovery and communication

## Flow

1. The client app uses `actr_ref.call()` to invoke `StreamClient.StartStream` on the local workload.
2. The local handler discovers a `DataStreamPeerConcurrentServer` actor and calls `StreamServer.PrepareStream`.
3. The server handler calls `StreamClient.PrepareClientStream` so the client registers a DataStream callback.
4. The server sends DataStream chunks back to the client.
5. The client handler sends DataStream chunks with `ctx.send_data_stream()`.

## Proto

`proto/data_stream_peer.proto` defines two services:

- `StreamClient.StartStream`
- `StreamServer.PrepareStream`

## Run

1. Start the signaling server:

```bash
cargo run -p signaling-server
```

2. Generate code:

```bash
cd data-stream-peer-concurrent/shared
actr gen -i ../proto
```

3. Copy configs:

```bash
cp server/Actr.example.toml server/actr.toml
cp client/Actr.example.toml client/actr.toml
```

4. Start the server:

```bash
cd server
cargo run
```

5. Run the client:

```bash
cd client
cargo run -- client-1 5
```
