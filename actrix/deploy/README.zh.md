# 部署助手

用 Rust 编写的现代化、交互式 Actrix 辅助服务部署助手。

## 功能特性

- 🚀 **交互式命令行**: 使用 `dialoguer` 提供用户友好的菜单和提示
- ⚙️ **服务选择**: 选择启用的服务（信令、STUN、TURN、Ais）
- 🌐 **网络配置**: 自动 IP 检测和端口配置
- 🔒 **SSL 设置**: 自动化 SSL 证书配置
- 👥 **用户管理**: 自动创建用户/组并设置正确权限
- 📝 **模板处理**: 从模板生成配置文件
- 🔍 **系统检查**: 全面的依赖和兼容性检查
- 🎯 **完整向导**: 一键完成完整安装流程
- 🗂️ **智能安装**: 动态二进制检测和可配置路径
- 🛡️ **权限处理**: 需要时智能提升 sudo 权限
- ⚡ **输入缓冲清理**: 防止快速按键干扰后续输入
- 🔧 **Systemd 集成**: 完整的服务部署和状态监控
- 🔥 **防火墙引导**: 基于配置自动生成规则，确认后可一键应用或跳过

## 安装

作为 actrix 工作空间的一部分构建：

```bash
cargo build --release -p deploy
```

二进制文件将位于 `target/release/deploy`。

## 使用方法

### 交互式菜单（默认）
```bash
./deploy
# 或者
./deploy menu
```

### 配置向导
```bash
./deploy config
```

### 检查依赖
```bash
./deploy deps
```

### 安装应用程序文件
```bash
./deploy install
```

### 部署 systemd 服务
```bash
./deploy service
```

### 卸载应用程序
```bash
./deploy uninstall
```

### 完整安装
```bash
./deploy
# 然后选择"完整安装向导"
```

## 选项参数

- `--debug`: 启用调试模式（显示将要执行的操作但不实际执行）
- `--config <PATH>`: 指定配置文件路径（默认：`/etc/actrix/config.toml`）
- `--install-dir <PATH>`: 设置安装目录（默认：`/opt/actrix`）
- `--binary-name <NAME>`: 设置二进制文件名（默认：`auxes`）
- `--add-to-path`: 添加二进制文件符号链接到系统 PATH

## 架构设计

部署助手按以下模块组织：

- **`cli/`**: 子命令定义（`deps/install/service/uninstall`）
- **`config/`**: 安装路径配置（`install_config.rs`）
- **`system/`**: 系统操作和引导流程
  - `install.rs`: 二进制安装与 systemd 部署
  - `uninstall.rs`: 卸载流程
  - `dependencies.rs`: 系统依赖检查
- **`tpl/`**: 模板渲染逻辑（`systemd_service.rs`，模板内联）

## 服务配置

该工具支持配置以下服务：

- **信令服务**（位 1）: WebSocket 信令服务
- **STUN**（位 2）: 用于 NAT 穿越的 STUN 服务器
- **TURN**（位 4）: TURN 中继服务器（包含 STUN 功能）
- **Ais 服务**（位 8）: ActorRTC 身份服务

服务根据选择自动配置：
- 仅当选择信令或 Ais 服务时才提示 HTTP/HTTPS 配置
- 仅当选择 STUN 或 TURN 时才提示 ICE 端口配置
- 仅当选择 TURN 时才提示 TURN 域配置
- 用户/组配置根据服务选择提供适当的默认值

## 模板系统

该工具使用基于模板的配置系统：
- Systemd 服务模板与渲染逻辑位于 `src/tpl/systemd_service.rs`（内联常量）
- 模板使用占位符替换（`{{VARIABLE}}`）语法
- 自动创建目录并设置正确权限
- 需要时使用 sudo 安全写入文件

## 用户管理

在 Unix 系统上，该工具可以：
- 检测现有用户和组
- 创建具有适当设置的系统用户
- 创建组并将用户添加到组
- 在安装目录上设置正确的权限

## 开发指南

要扩展部署助手：

1. 在适当的模块中添加新功能
2. 在 `main.rs` 中更新 CLI 命令
3. 为新的验证函数添加测试
4. 使用新功能更新此 README

## 卸载流程

卸载向导提供对移除内容的精细控制：

1. **Systemd 服务**: 停止并移除服务文件
2. **应用程序文件**: 移除 `/opt/actrix` 目录
3. **配置文件**: 可选移除 `/etc/actrix`（默认保留）
4. **系统用户/组**: 可选移除 `actrix` 用户和组

每个组件都可以单独选择移除，允许您：
- 保留配置同时移除二进制文件
- 保留用户账户以便将来重新安装
- 选择性清理特定组件

## 注意事项

本助手的主要目标是帮助新手用户快速上手。部分配置项未在工具界面中直接体现，包括：
- 用于本地调试的 http 选项配置
- 用于受管模式的 admin 相关配置
请结合文档和 example，直接在 .config 文件中完成相关配置。
