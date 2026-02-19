# Actrix 部署引导（最小版）

`deploy/` 目录现在只保留最小引导职责：依赖检查、二进制安装、systemd 服务部署、卸载。

## 约束

- `service` 子命令仅支持 `systemd` 环境，会先检测后再执行。
- 安装目录禁止位于 `/home` 或 `/tmp`，命中会直接报错并终止。

## 命令

```bash
# 依赖检查
cargo run --manifest-path deploy/Cargo.toml -- deps

# 安装二进制到系统目录
cargo run --manifest-path deploy/Cargo.toml -- install

# 安装并启动 systemd 服务
cargo run --manifest-path deploy/Cargo.toml -- service

# 卸载
cargo run --manifest-path deploy/Cargo.toml -- uninstall
```

## 目录说明

- `src/`：部署引导代码（CLI + systemd/安装逻辑）
- `install.sh`：脚本化安装入口
