# Actrix 部署工具（actrix-deploy）

`actrix-deploy` 是 actrix 服务器部署的唯一入口：从 GitHub Release 或本地二进制下载、
校验、安装、升级和回滚 actrix，并部署 systemd 服务。不依赖目标机器上的源码或
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

## 命令

```bash
# 依赖检查
actrix-deploy deps

# 从 GitHub Release 首次安装
sudo actrix-deploy install --tag v0.4.3 \
  --install-dir /opt/actrix --config /etc/actrix/config.toml \
  --service-name actrix2 --user actor-rtc --group actor-rtc

# 从本地二进制安装（离线/灰度，需 --sha256-path 或 --skip-verify）
sudo actrix-deploy install --binary-path ./actrix-linux-x86_64 \
  --sha256-path ./actrix-linux-x86_64.sha256 --version v0.4.3 \
  --install-dir /opt/actrix --config /etc/actrix/config.toml

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
