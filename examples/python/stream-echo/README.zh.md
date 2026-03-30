# Stream Echo Example

这是一个演示 Stream 消息传输的示例，展示了客户端如何注册 stream 并接收服务端推送的消息。

## 功能说明

1. **Client** 定义本地服务 `LocalStreamService`（包含 `StartStream` RPC）
2. 在 `StartStream` 服务中：
   - 发现 Server
   - 注册 stream 回调 (`ctx.register_stream`)
   - 调用 Server 的 `RegisterStream` RPC
3. **Server** 接收 `RegisterStream` 请求后：
   - 记录 `stream_id` 和客户端信息
   - 返回成功响应
   - 启动后台任务向 Client 发送指定数量的 stream 消息
4. **Client** 通过之前注册的回调处理接收到的 stream 消息

## 目录结构

```
stream-echo/
├── server/
│   ├── protos/
│   │   └── local/
│   │       └── stream_server.proto    # Server 服务定义
│   ├── actr.toml                      # Server 配置
│   ├── manifest.lock.toml                # 依赖锁文件
│   ├── server.py                      # Server 主程序
│   └── stream_server.py              # Server 服务实现（生成的 scaffold）
│
└── client/
    ├── protos/
    │   ├── local/
    │   │   └── stream_client.proto    # Client 本地服务定义
    │   └── remote/
    │       └── stream-register-server-python/
    │           └── stream_server.proto  # Server 服务定义（依赖）
    ├── actr.toml                      # Client 配置
    ├── manifest.lock.toml                # 依赖锁文件
    └── client.py                      # Client 实现
```

## 运行示例

### 1. 启动 Server

```bash
cd server

# 安装依赖（如果需要）
actr install

# 生成代码
actr gen --input protos --output generated --language python

# 运行 server
python server.py --actr-toml actr.toml
```

### 2. 启动 Client

在另一个终端：

```bash
cd client

# 安装依赖
actr install

# 生成代码
actr gen --input protos --output generated --language python

# 运行 client（参数：stream_id 消息数量）
python client.py --actr-toml actr.toml <stream_id> <message_count>

# 示例：接收 10 条消息
python client.py --actr-toml actr.toml my-stream 10
```

参数说明：
- `<stream_id>`: Stream 标识符（字符串）
- `<message_count>`: 要接收的 stream 消息数量

## 流程说明

1. **Client 启动**：
   - 从命令行参数获取 `stream_id` 和 `message_count`
   - 调用本地的 `StartStream` 服务

2. **StartStream 服务内部**：
   - 发现 Server（通过 `ctx.discover`）
   - **先注册** stream 回调函数
   - 发送 `RegisterStream` 请求给 Server

3. **Server 处理请求**：
   - 记录 stream_id 和客户端 ID
   - 返回成功响应
   - 启动后台任务，循环发送指定数量的 stream 消息

4. **Client 接收消息**：
   - 通过注册的回调函数接收并处理每条 stream 消息
   - 打印接收到的消息内容

## 关键代码

### Client 端 - StartStream Handler

```python
class LocalStreamService(local_service_actor.LocalStreamServiceHandler):
    async def start_stream(self, req, ctx):
        stream_id = req.stream_id
        message_count = req.message_count
        
        # 1. 发现 Server
        server_id = await ctx.discover(self.server_type)
        
        # 2. 先注册回调
        async def stream_callback(stream: DataStream, sender_id):
            text = stream.payload().decode("utf-8")
            logger.info("📨 Received: %s", text)
        
        await ctx.register_stream(stream_id, stream_callback)
        
        # 3. 调用 Server 的 RegisterStream
        register_req = server_pb2.RegisterStreamRequest(
            stream_id=stream_id,
            message_count=message_count,
        )
        response = await ctx.call(
            Dest.actor(server_id),
            register_req.route_key,
            register_req,
        )
        
        return pb2.StartStreamResponse(
            success=response.success,
            message=response.message,
        )
```

### Server 端 - RegisterStream Handler

```python
async def register_stream(self, req, ctx):
    stream_id = req.stream_id
    message_count = req.message_count
    caller = ctx.caller_id()
    
    # 启动后台任务发送 stream 消息
    async def _send_stream_messages():
        for i in range(1, message_count + 1):
            message = f"[server] Stream message {i} for {stream_id}"
            data_stream = DataStream(
                stream_id=stream_id,
                sequence=i,
                payload=message.encode("utf-8"),
            )
            await ctx.send_stream(Dest.actor(caller), data_stream)
            await asyncio.sleep(1.0)
    
    asyncio.create_task(_send_stream_messages())
    
    return RegisterStreamResponse(
        success=True,
        message=f"Stream {stream_id} registered successfully",
    )
```

## 代码生成说明

### Server

```bash
cd server
actr gen --input protos --output generated --language python
```

生成的文件：
- `generated/local/stream_server_pb2.py` - Protobuf 消息定义
- `generated/stream_server_actor.py` - Actor 基础代码（Handler, Dispatcher）
- `stream_server.py` - 服务实现 scaffold（需要实现业务逻辑）

### Client

```bash
cd client
actr gen --input protos --output generated --language python
```

生成的文件：
- `generated/local/stream_client_pb2.py` - 本地服务的 Protobuf 定义
- `generated/local_stream_service_actor.py` - 本地服务的 Actor 代码
- `generated/remote/stream_register_server_python/stream_server_pb2.py` - 远程服务定义
- `generated/remote/stream_register_server_python/stream_server_client.py` - 远程服务客户端

## 注意事项

- Client 的 `LocalStreamService` **不在 actr.toml 的 exports 中**，只供本地调用
- Client 必须在发送 `RegisterStream` 请求**之前**调用 `ctx.register_stream` 注册回调
- Server 根据请求中的 `message_count` 发送相应数量的 stream 消息
- Stream 消息是单向的，不需要响应
- 所有生成的代码都在 `generated/` 目录下：
  - `generated/local/` - 本地 proto 生成的代码
  - `generated/remote/` - 远程依赖生成的代码
  - `generated/*_actor.py` - Actor 基础代码在根目录

## 依赖管理

### Server 配置 (actr.toml)

```toml
exports = ["protos/local/stream_server.proto"]

[package]
name = "stream-register-server-python"

[actr_type]
manufacturer = "acme"
name = "StreamEchoServer"
```

### Client 配置 (actr.toml)

```toml
[dependencies]
stream-register-server-python = { actr_type = "acme+StreamEchoServer" }

[package]
name = "stream-register-client-python"

[actr_type]
manufacturer = "acme"
name = "StreamEchoClient"
```

运行 `actr install` 会：
1. 从 Signaling Server 下载远程服务的 proto 文件
2. 生成 `manifest.lock.toml` 锁定依赖版本
3. 将远程 proto 文件缓存到 `protos/remote/` 目录
