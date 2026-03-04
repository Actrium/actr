# actr-web-protoc-codegen

Protoc 代码生成器，用于从 Protobuf 定义生成 actr-web 代码。

## 功能

- ✅ 从 `.proto` 文件生成 Rust WASM Actor 代码
- ✅ 生成 TypeScript 类型定义
- ✅ 生成 TypeScript ActorRef 包装类
- ✅ 可选：生成 React Hooks
- 🔄 自动化代码格式化
- 🔄 支持自定义模板

## 使用方式

### 方式 1：在 build.rs 中使用（推荐）

```rust
// build.rs
fn main() {
    use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

    let config = WebCodegenConfig::builder()
        .proto_file("proto/echo.proto")
        .rust_output("src/generated")
        .ts_output("../packages/web-sdk/src/generated")
        .with_react_hooks(true)
        .include("proto")
        .build()
        .expect("Invalid config");

    WebCodegen::new(config)
        .generate()
        .expect("Failed to generate code");

    println!("cargo:rerun-if-changed=proto");
}
```

### 方式 2：通过 actr-cli 使用

```bash
# 安装支持 web 的 actr-cli
cargo install actr-cli --features web

# 生成代码
actr gen --platform web \
  --input proto/ \
  --output crates/actors/src/generated/ \
  --ts-output packages/web-sdk/src/generated/ \
  --react-hooks
```

### 方式 3：编程式 API

```rust
use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

let config = WebCodegenConfig {
    proto_files: vec!["proto/echo.proto".into()],
    rust_output_dir: "src/generated".into(),
    ts_output_dir: "../web-sdk/src/generated".into(),
    generate_react_hooks: true,
    includes: vec!["proto".into()],
    format_code: true,
    custom_templates_dir: None,
};

let codegen = WebCodegen::new(config);
let files = codegen.generate()?;

// 写入文件
files.write_to_disk()?;
```

## 生成的代码结构

### Rust 侧（WASM）

```
src/generated/
├── mod.rs
├── echo.rs          # EchoActor
└── ...
```

### TypeScript 侧

```
src/generated/
├── index.ts
├── echo.types.ts         # 类型定义
├── echo.actor-ref.ts     # EchoActorRef 类
├── use-echo.ts           # useEcho Hook (可选)
└── ...
```

## 配置选项

| 选项 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `proto_files` | `Vec<PathBuf>` | ✅ | Proto 文件列表 |
| `rust_output_dir` | `PathBuf` | ✅ | Rust 输出目录 |
| `ts_output_dir` | `PathBuf` | ✅ | TypeScript 输出目录 |
| `generate_react_hooks` | `bool` | ❌ | 是否生成 React Hooks（默认 false） |
| `includes` | `Vec<PathBuf>` | ❌ | Proto include 路径 |
| `format_code` | `bool` | ❌ | 是否格式化代码（默认 false） |
| `custom_templates_dir` | `Option<PathBuf>` | ❌ | 自定义模板目录 |

## 开发状态

- [x] 基础架构和配置
- [x] 完整的 Proto 解析（手写 parser）
- [x] Rust Actor 方法生成
- [x] TypeScript 类型生成
- [x] TypeScript ActorRef 方法生成
- [x] React Hooks 生成
- [x] 流式方法支持
- [x] 代码格式化集成（rustfmt + prettier/dprint）
- [x] 单元测试
- [ ] prost-build 集成（可选，当前使用手写 parser）
- [ ] 自定义模板支持
- [ ] 集成测试
- [ ] 性能优化

## 示例

参考 `examples/` 目录下的示例项目：

- `examples/echo/` - 基础 Echo 服务示例
- `examples/simple-rpc/` - 端到端 RPC 示例

## 许可证

Apache License 2.0
