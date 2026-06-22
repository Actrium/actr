# Actrix 部署引导（最小版）

`deploy/` 目录现在只保留最小引导职责：依赖检查、二进制安装、systemd 服务部署、卸载。

## 约束

- `service` 子命令仅支持 `systemd` 环境，会先检测后再执行。
- 安装目录禁止位于 `/home` 或 `/tmp`，命中会直接报错并终止。
- `service` 部署时会根据 `config.toml` 生成防火墙规则，支持用户确认后自动应用或跳过。

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

```bash
sudo ./deploy/install.sh install
```

安装脚本会：
- 创建系统用户 `actrix:actrix`
- 安装二进制文件到 `/opt/actrix/`
- 复制配置文件到 `/opt/actrix/config.toml`
- 安装 systemd 服务

### 3. 配置服务

编辑配置文件：

```bash
sudo nano /opt/actrix/config.toml
```

关键配置项：
- `enable`: 服务启用位掩码
- `actrix_shared_key`: **必须修改默认值！**
- `bind.https`: HTTPS 证书和端口配置
- `bind.ice`: STUN/TURN 端口配置
- `turn.public_ip`: TURN 服务器的公网 IP
- `log_output`: 设置为 "file" 并启用 log_rotate

### 4. 启动服务

```bash
sudo systemctl enable actrix
sudo systemctl start actrix
sudo systemctl status actrix
sudo journalctl -u actrix -f
```

## 更新

```bash
git pull
cargo build --release
sudo ./deploy/install.sh update
```

## 卸载

```bash
sudo ./deploy/install.sh uninstall
```

完整文档请访问: https://github.com/Actrium/actrix
