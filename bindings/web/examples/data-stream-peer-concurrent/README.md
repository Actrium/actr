# Data Stream Peer Concurrent - Web Example

这个示例把 [examples/rust/data-stream-peer-concurrent](../../../../examples/rust/data-stream-peer-concurrent/README.md) 的核心流程搬到了浏览器端：

- 浏览器 Client Actor 发起 `StartStream`
- Client 在本地 WASM handler 中发现远端 Server Actor
- Server 调回 Client 的 `PrepareClientStream`
- 双方分别调用 `ctx.register_stream()` 注册 `stream_id`
- Client / Server 通过 `ctx.send_data_stream()` 在 WebRTC DataChannel 上双向发送数据
- `test-auto.js` 用 Puppeteer 同时拉起多个 Client 页面，验证并发场景

## 目录结构

```text
data-stream-peer-concurrent/
├── client/
│   ├── public/actor.sw.js
│   ├── src/
│   └── wasm/
├── server/
│   ├── public/actor.sw.js
│   ├── src/
│   └── wasm/
├── package.json
├── start.sh
└── test-auto.js
```

## 前置要求

1. 在 `actrix` 仓库启动 signaling 服务，监听：`ws://127.0.0.1:8081/signaling/ws`
2. 安装：Node.js 18+、pnpm、Rust 1.88+、wasm-pack

## 运行方式

```bash
cd bindings/web/examples/data-stream-peer-concurrent
./start.sh
```

`start.sh` 会：

1. 安装测试/示例依赖
2. 编译 client/server WASM
3. 启动两个 Vite dev server
4. 运行 `test-auto.js` 做并发验证

## 单独跑自动化测试

```bash
cd bindings/web/examples/data-stream-peer-concurrent
pnpm install
pnpm install --dir client
pnpm install --dir server
./client/build.sh
./server/build.sh

# 终端 1
cd client && pnpm dev --host 127.0.0.1 --port 4175

# 终端 2
cd server && pnpm dev --host 127.0.0.1 --port 4176

# 终端 3
node test-auto.js
```

## 验证点

- Client 页面能看到：
	- `start_stream response`
	- `client sending N/N`
	- `client received N/N`
- Server 页面能看到：
	- `prepare_stream`
	- `server: stream <stream_id> received N/N`
	- `server sending N/N`

## 说明

- 这里的 RPC 请求体使用 JSON 编码，重点验证的是 web 侧 `register_stream()` / `send_data_stream()` / Fast Path 分发链路，而不是 protobuf codegen。
- 并发测试默认启动 2 个 Client 页面，每个页面发送 3 条消息；可通过环境变量覆盖：
	- `CLIENT_COUNT`
	- `MESSAGE_COUNT`
	- `CLIENT_URL`
	- `SERVER_URL`
