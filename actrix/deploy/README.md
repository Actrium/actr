# Actrix 部署工具（actrix-deploy）

`actrix-deploy` 是 actrix 二进制安装与 systemd 部署的统一工具：从 GitHub Release 下载或
使用本地二进制，校验、安装、升级和回滚 actrix，并部署 systemd 服务。不依赖目标机器上的源码或
Rust toolchain。

## 安装模型

版本化二进制放在 `releases/<version>/actrix`，活跃版本通过 `bin/actrix` 软链接指向：

```text
/opt/actrix/releases/v0.4.3/actrix
/opt/actrix/releases/v0.4.4/actrix
/opt/actrix/bin/actrix -> /opt/actrix/releases/v0.4.4/actrix
/opt/actrix/shared/   # 运行时共享数据
/opt/actrix/logs/     # 日志
/opt/actrix/db/       # SQLite 数据
```

systemd 的 `ExecStart` 固定指向 `bin/actrix`，所以切换版本只需重指软链接 + 重启服务，
**永不修改 systemd unit**。配置、数据库、日志、证书放在版本目录之外，切换版本不影响状态。

## 安装 actrix-deploy

从 GitHub Release 下载（首个版本可人工下载）：

```bash
curl -LO https://github.com/Actrium/actr/releases/download/v0.4.3/actrix-deploy-linux-x86_64
curl -LO https://github.com/Actrium/actr/releases/download/v0.4.3/actrix-deploy-linux-x86_64.sha256
sha256sum -c actrix-deploy-linux-x86_64.sha256
sudo install -m 0755 actrix-deploy-linux-x86_64 /usr/local/sbin/actrix-deploy
```

## 新部署一套 actrix 服务（完整流程）

在一台全新机器上从零部署单实例。多实例见末尾。

### 前置条件

- systemd Linux（Ubuntu 24.04 已验证），x86_64 或 arm64，root 权限
- 开放端口：HTTP（如 8080）、HTTPS（如 443）、ICE/TURN（3478）、relay 范围（49152-65535）
- 域名 DNS 指向本机 + TLS 证书
- 准备好：`actrix-deploy` 工具、`actrix` 二进制、`config.toml`、证书

### 1. 安装 actrix-deploy 工具

见上方「安装 actrix-deploy」；或从构建机 `scp` 任意 `deploy` 二进制到
`/usr/local/bin/actrix-deploy`（二进制名是 `deploy`，安装时可重命名）。

### 2. 准备配置与证书

建议都放在安装目录 `/opt/actrix` 下，这样 `WorkingDirectory` 默认即此目录，
config 中的相对路径自动正确：

```text
/opt/actrix/
├── config.toml
├── certificates/
│   ├── your.domain.crt
│   └── your.domain.key
└── (database/ signer.db 运行时自动生成)
```

`config.toml` 中 `cert`/`key`/`sqlite_path`/`signer.db` 可用相对路径（相对
`/opt/actrix`）。各 secret（`actrix_shared_key`、`renewal_token_secret`、
`shared_secret`、admin `password`）请用本机随机值。

### 3. 安装 actrix 二进制

```bash
# 方式 A：从 GitHub Release（需该 tag 带 .sha256 sidecar）
sudo actrix-deploy install --tag v0.4.3 --install-dir /opt/actrix --no-path

# 方式 B：本地二进制（现有 release 尚无 sidecar 时推荐）
sudo actrix-deploy install --binary-path /root/actrix \
  --version v0.4.3 --sha256-path /root/actrix.sha256 \
  --install-dir /opt/actrix --no-path
# 测试可加 --skip-verify 省去 sha256
```

生成 `/opt/actrix/releases/v0.4.3/actrix` 与 `/opt/actrix/bin/actrix` 软链。

### 4. 创建 systemd 服务并启动

```bash
sudo actrix-deploy service \
  --service-name actrix --install-dir /opt/actrix \
  --config /opt/actrix/config.toml \
  --user actrix --group actrix \
  --working-directory /opt/actrix \
  --force-overwrite-unit
```

工具会：检测 systemd → 生成加固 unit（`bin/actrix` 入口；端口 <1024 自动加
`CAP_NET_BIND_SERVICE`；`ProtectSystem=strict`；`ReadWritePaths` 按 config 自动算）
→ 创建用户/组 → daemon-reload → enable → start；并询问是否应用 ufw 防火墙规则。

> **非 root 用户**：`service` 会创建 `actrix` 用户但不会 chown 文件。启动前需
> ```bash
> sudo chown -R actrix:actrix /opt/actrix
> ```
> 否则非 root 进程读不了证书、写不了 db。先跑通用 `--user root --group root` 亦可，
> 日后再降权。
>
> **`--working-directory`**：当 config 的相对路径相对的是安装目录之外的目录时
> （如历史部署的 `/opt/actr-project/actrix`），用它指定真实工作目录；相对路径与
> `ReadWritePaths` 都按它解析。全新部署把文件都放 `/opt/actrix` 下则无需此参数。

### 5. 验证

```bash
actrix-deploy status --install-dir /opt/actrix     # 当前版本 + 软链 + 已装版本
systemctl is-active actrix                          # active
journalctl -u actrix -f                             # 日志
curl -k https://localhost/ais/health                # 健康检查（端口见 config）
```

### 升级 / 回滚

```bash
sudo actrix-deploy update   --tag v0.4.4 --install-dir /opt/actrix --restart-service actrix
sudo actrix-deploy rollback --to v0.4.3  --install-dir /opt/actrix --restart-service actrix
```

升级失败自动回滚到上一版本。

### 多实例（一台机器跑多套）

用不同 `--service-name` + 不同 `--install-dir` + 不同 config（端口不可冲突）：

```bash
sudo actrix-deploy install --tag v0.4.3 --install-dir /opt/actrix-a --no-path
sudo actrix-deploy service --service-name actrix-a --install-dir /opt/actrix-a \
  --config /opt/actrix-a/config.toml --user actrix --group actrix --working-directory /opt/actrix-a

sudo actrix-deploy install --tag v0.4.3 --install-dir /opt/actrix-b --no-path
sudo actrix-deploy service --service-name actrix-b --install-dir /opt/actrix-b \
  --config /opt/actrix-b/config.toml --user actrix --group actrix --working-directory /opt/actrix-b
```

两个 unit 名不同，各自独立升级/回滚，互不影响。

## 命令

```bash
# 依赖检查
actrix-deploy deps

# 从 GitHub Release 首次安装
sudo actrix-deploy install --tag v0.4.3 --install-dir /opt/actrix --no-path

# 从本地二进制安装（离线/灰度，需 --sha256-path 或 --skip-verify）
sudo actrix-deploy install --binary-path ./actrix-linux-x86_64 \
  --sha256-path ./actrix-linux-x86_64.sha256 --version v0.4.3 \
  --install-dir /opt/actrix --no-path

# 开发：用本地 target/release/actrix 构建
sudo actrix-deploy install --from-local-build --install-dir /opt/actrix

# 部署/重建 systemd 服务（已存在默认拒绝覆盖，--force-overwrite-unit 才覆盖）
sudo actrix-deploy service --service-name actrix2 --install-dir /opt/actrix \
  --config /etc/actrix/config.toml --user actor-rtc --group actor-rtc

# 升级（切软链接 + 可选重启；失败自动回滚到上一版本）
sudo actrix-deploy update --tag v0.4.4 --install-dir /opt/actrix --restart-service actrix2

# 回滚到已安装的版本
sudo actrix-deploy rollback --to v0.4.3 --install-dir /opt/actrix --restart-service actrix2

# 查看当前版本与已安装版本
actrix-deploy status --install-dir /opt/actrix

# 卸载（分组确认；默认保留 db/logs/shared 和配置）
sudo actrix-deploy uninstall --install-dir /opt/actrix --service-name actrix2
```

## 二进制来源与校验

`install`/`update` 三选一：`--tag`、`--latest`、`--binary-path`。

- Release 模式（`--tag`/`--latest`）：必须下载并校验 `.sha256`，缺失或不一致即失败。
- 本地模式（`--binary-path`）：默认要求 `--sha256-path`；`--skip-verify` 可跳过（打印强警告，不用于生产）。

## 环境变量

| 变量 | 作用 |
|------|------|
| `GITHUB_TOKEN` | 私有仓库下载用，仅需 Contents Read 权限。 |
| `ACTRIX_REPOSITORY` | GitHub owner/repo，默认 `Actrium/actr`。 |
| `ACTRIX_HEALTH_WAIT_SECONDS` | `update`/`rollback` 重启后等待服务 active 的秒数，默认 5。 |

## 约束

- `service` 仅支持 systemd，先检测再执行。
- 安装目录禁止位于 `/home` 或 `/tmp`。
- `update` 永不写 systemd unit；unit 加固由 `service`/人工运维维护。
- 服务名必须显式传入（`--service-name`），不靠默认值猜，以便单机多实例。

## 可选配置文件

`/etc/actrix/deploy.toml`（计划中，当前未自动加载；CLI 参数与环境变量为准）示例见
`deploy.toml.example`。
