//! 统一配置管理向导
//!
//! 使用 config crate 定义的统一配置结构，通过交互式方式生成配置文件

use crate::system::{NetworkUtils, clear_input_buffer, validate_port};
use actrix_common::config::bind::{HttpBindConfig, HttpsBindConfig};
use actrix_common::config::supervisor::{SupervisorClientConfig, SupervisordConfig};
use actrix_common::config::{self, ActrixConfig, BindConfig, SupervisorConfig, TurnConfig};
use anyhow::{Context, Result};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

/// 统一配置向导
pub struct UnifiedConfigWizard {
    debug: bool,
    theme: ColorfulTheme,
}

impl UnifiedConfigWizard {
    pub fn new(debug: bool) -> Self {
        Self {
            debug,
            theme: ColorfulTheme::default(),
        }
    }

    /// 运行配置向导，返回生成的配置文件路径
    pub fn run(&mut self) -> Result<PathBuf> {
        println!("🚀 Actor-RTC 辅助服务配置向导");
        println!("═══════════════════════════════");
        println!("使用统一配置管理系统，基于类型安全的配置结构");
        println!();

        // 第1步：选择配置文件位置
        let output_path = self.choose_config_location()?;

        // 第2步：读取模板文件
        let template_content = self.load_template()?;

        // 第3步：交互式配置收集
        let config = self.collect_configuration()?;

        // 第4步：生成最终配置文件
        self.generate_config_file(&config, &template_content, &output_path)?;

        println!("✅ 配置文件生成成功！");
        println!("📄 文件位置: {}", output_path.display());

        Ok(output_path)
    }

    /// 加载配置模板文件
    fn load_template(&self) -> Result<String> {
        let template_path = Path::new("tpl/config.template.toml");
        if !template_path.exists() {
            anyhow::bail!("配置模板文件不存在: {}", template_path.display());
        }

        std::fs::read_to_string(template_path)
            .with_context(|| format!("无法读取配置模板文件: {}", template_path.display()))
    }

    /// 交互式收集配置信息
    fn collect_configuration(&self) -> Result<ActrixConfig> {
        let mut config = ActrixConfig::default();

        // 服务选择
        self.configure_services(&mut config)?;

        // 基本系统配置
        self.configure_basic_settings(&mut config)?;

        // 网络配置
        self.configure_network(&mut config)?;

        // 条件性配置
        if config.is_turn_enabled() {
            self.configure_turn(&mut config)?;
        }

        if self.needs_supervisor(&config) {
            self.configure_supervisor(&mut config)?;
        }

        Ok(config)
    }

    /// 配置启用的服务
    fn configure_services(&self, config: &mut ActrixConfig) -> Result<()> {
        println!("📋 服务选择");
        println!("===========");

        let service_options = vec![
            ("Signaling (信令服务)", config::ENABLE_SIGNALING),
            ("STUN (NAT 发现)", config::ENABLE_STUN),
            ("TURN (流量中继)", config::ENABLE_TURN),
            ("AIS (身份认证服务)", config::ENABLE_AIS),
        ];

        let mut enable_mask = 0u8;

        for (service_name, mask) in &service_options {
            let enabled = Confirm::with_theme(&self.theme)
                .with_prompt(format!("启用 {}", service_name))
                .default(true)
                .interact()?;

            if enabled {
                enable_mask |= mask;
            }
        }

        if enable_mask == 0 {
            anyhow::bail!("至少需要启用一个服务");
        }

        config.enable = enable_mask;
        println!(
            "✅ 启用的服务: 0b{:05b} (十进制: {})",
            enable_mask, enable_mask
        );
        println!();

        Ok(())
    }

    /// 配置基本系统设置
    fn configure_basic_settings(&self, config: &mut ActrixConfig) -> Result<()> {
        println!("⚙️  基本设置");
        println!("============");

        // 服务器名称
        config.name = Input::with_theme(&self.theme)
            .with_prompt("服务器实例名称")
            .default(config.name.clone())
            .interact_text()?;

        // 运行环境
        let env_options = vec!["dev", "prod", "test"];
        let env_index = Select::with_theme(&self.theme)
            .with_prompt("运行环境")
            .items(&env_options)
            .default(0)
            .interact()?;
        config.env = env_options[env_index].to_string();

        // 位置标签
        config.location_tag = Input::with_theme(&self.theme)
            .with_prompt("位置标签")
            .default(config.location_tag.clone())
            .interact_text()?;

        // 日志级别
        let log_levels = vec!["trace", "debug", "info", "warn", "error"];
        let log_index = Select::with_theme(&self.theme)
            .with_prompt("日志级别")
            .items(&log_levels)
            .default(2) // info
            .interact()?;
        config.observability.filter_level = log_levels[log_index].to_string();

        // 数据库路径
        let sqlite_path_str = Input::with_theme(&self.theme)
            .with_prompt("SQLite 数据库存储目录路径")
            .default(config.sqlite_path.display().to_string())
            .interact_text()?;
        config.sqlite_path = PathBuf::from(sqlite_path_str);

        // 运行用户（可选）
        let use_custom_user = Confirm::with_theme(&self.theme)
            .with_prompt("配置运行用户和组")
            .default(false)
            .interact()?;

        if use_custom_user {
            let user: String = Input::with_theme(&self.theme)
                .with_prompt("运行用户")
                .default("actor-rtc".to_string())
                .interact_text()?;
            config.user = Some(user);

            let group: String = Input::with_theme(&self.theme)
                .with_prompt("运行用户组")
                .default("actor-rtc".to_string())
                .interact_text()?;
            config.group = Some(group);
        }

        println!();
        Ok(())
    }

    /// 配置网络设置
    fn configure_network(&self, config: &mut ActrixConfig) -> Result<()> {
        println!("🌐 网络配置");
        println!("===========");

        // 选择服务器地址
        let server_host = self.select_server_host()?;

        // 配置 HTTP 绑定（如果需要）
        if self.needs_http_services(config) {
            let use_http = if config.env == "dev" {
                Confirm::with_theme(&self.theme)
                    .with_prompt("启用 HTTP 服务（开发环境）")
                    .default(true)
                    .interact()?
            } else {
                false
            };

            if use_http {
                let http_port = self.prompt_port("HTTP 端口", 8080)?;
                config.bind.http = Some(HttpBindConfig {
                    domain_name: "localhost".to_string(),
                    advertised_ip: server_host.clone(),
                    ip: "0.0.0.0".to_string(),
                    port: http_port,
                });
            }
        }

        // 配置 HTTPS 绑定（生产环境必需）
        if self.needs_http_services(config) {
            let use_https = if config.env == "prod" {
                true
            } else {
                Confirm::with_theme(&self.theme)
                    .with_prompt("启用 HTTPS 服务")
                    .default(false)
                    .interact()?
            };

            if use_https {
                let https_port = self.prompt_port("HTTPS 端口", 8443)?;

                let cert_path: String = Input::with_theme(&self.theme)
                    .with_prompt("SSL 证书文件路径")
                    .default("certificates/server.crt".to_string())
                    .interact_text()?;

                let key_path: String = Input::with_theme(&self.theme)
                    .with_prompt("SSL 私钥文件路径")
                    .default("certificates/server.key".to_string())
                    .interact_text()?;

                config.bind.https = Some(HttpsBindConfig {
                    domain_name: "localhost".to_string(),
                    advertised_ip: server_host.clone(),
                    ip: "0.0.0.0".to_string(),
                    port: https_port,
                    cert: cert_path,
                    key: key_path,
                });
            }
        }

        // 配置 ICE 绑定（如果需要）
        if config.is_ice_enabled() {
            let ice_port = self.prompt_port("ICE 端口 (STUN/TURN)", 3478)?;
            config.bind.ice.ip = "0.0.0.0".to_string();
            config.bind.ice.port = ice_port;
        }

        println!();
        Ok(())
    }

    /// 配置 TURN 服务
    fn configure_turn(&self, config: &mut ActrixConfig) -> Result<()> {
        println!("🔄 TURN 服务配置");
        println!("================");

        config.turn.advertised_ip = Input::with_theme(&self.theme)
            .with_prompt("TURN 公网 IP 地址")
            .default(config.turn.advertised_ip.clone())
            .interact_text()?;

        config.turn.advertised_port =
            self.prompt_port("TURN 公网端口", config.turn.advertised_port)?;

        config.turn.realm = Input::with_theme(&self.theme)
            .with_prompt("TURN 认证域")
            .default(config.turn.realm.clone())
            .interact_text()?;

        config.turn.relay_port_range = Input::with_theme(&self.theme)
            .with_prompt("中继端口范围 (格式: 开始-结束)")
            .default(config.turn.relay_port_range.clone())
            .interact_text()?;

        println!();
        Ok(())
    }

    /// 配置 Supervisor 集成
    fn configure_supervisor(&self, config: &mut ActrixConfig) -> Result<()> {
        println!("👥 Supervisor 平台集成");
        println!("======================");

        let node_id: String = Input::with_theme(&self.theme)
            .with_prompt("节点 ID")
            .interact_text()?;

        let endpoint: String = Input::with_theme(&self.theme)
            .with_prompt("Supervisor gRPC 服务器地址")
            .default("http://localhost:50051".to_string())
            .interact_text()?;

        let shared_secret: String = Input::with_theme(&self.theme)
            .with_prompt("Shared Secret (64+ hex 字符)")
            .default("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string())
            .interact_text()?;

        let node_name: String = Input::with_theme(&self.theme)
            .with_prompt("节点展示名称（可留空使用 node_id）")
            .default(node_id.clone())
            .interact_text()?;

        let bind_ip: String = Input::with_theme(&self.theme)
            .with_prompt("Supervisord gRPC 绑定 IP")
            .default("0.0.0.0".to_string())
            .interact_text()?;

        let port = self.prompt_port("Supervisord gRPC 端口", 50055)?;

        let advertised_ip: String = Input::with_theme(&self.theme)
            .with_prompt("Supervisord 对外 IP/域名 (Supervisor 回连使用)")
            .default("127.0.0.1".to_string())
            .interact_text()?;

        let enable_tls: bool = Confirm::with_theme(&self.theme)
            .with_prompt("启用 TLS?")
            .default(false)
            .interact()?;

        let tls_domain = if enable_tls {
            Some(
                Input::with_theme(&self.theme)
                    .with_prompt("TLS 域名")
                    .interact_text()?,
            )
        } else {
            None
        };

        config.supervisor = Some(SupervisorConfig {
            connect_timeout_secs: 30,
            status_report_interval_secs: 60,
            health_check_interval_secs: 30,
            enable_tls,
            tls_domain,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            max_clock_skew_secs: 300,
            supervisord: SupervisordConfig {
                node_name,
                ip: bind_ip,
                port,
                advertised_ip,
            },
            client: SupervisorClientConfig {
                node_id,
                endpoint,
                shared_secret,
            },
        });

        println!();
        Ok(())
    }

    /// 生成最终配置文件
    fn generate_config_file(
        &self,
        config: &ActrixConfig,
        template: &str,
        output_path: &Path,
    ) -> Result<()> {
        let mut doc = template
            .parse::<DocumentMut>()
            .with_context(|| "解析配置模板失败")?;

        // 更新配置值
        self.update_config_document(&mut doc, config)?;

        // 写入文件
        if !self.debug {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
            }

            std::fs::write(output_path, doc.to_string())
                .with_context(|| format!("写入配置文件失败: {}", output_path.display()))?;
        } else {
            println!("🐛 调试模式: 配置文件内容:");
            println!("{}", doc.to_string());
        }

        Ok(())
    }

    /// 更新配置文档
    fn update_config_document(&self, doc: &mut DocumentMut, config: &ActrixConfig) -> Result<()> {
        // 基本配置
        doc["enable"] = value(config.enable as i64);
        doc["name"] = value(&config.name);
        doc["env"] = value(&config.env);
        doc["location_tag"] = value(&config.location_tag);
        doc["sqlite_path"] = value(config.sqlite_path.display().to_string().as_str());

        // 可观测性配置
        let mut observability_table = Table::new();
        observability_table["filter_level"] = value(&config.observability.filter_level);
        let mut log_table = Table::new();
        log_table["output"] = value(&config.observability.log.output);
        log_table["rotate"] = value(config.observability.log.rotate);
        log_table["path"] = value(&config.observability.log.path);
        observability_table["log"] = Item::Table(log_table);

        let tracing_cfg = &config.observability.tracing;
        let mut tracing_table = Table::new();
        tracing_table["enable"] = value(tracing_cfg.enable);
        tracing_table["service_name"] = value(&tracing_cfg.service_name);
        tracing_table["endpoint"] = value(&tracing_cfg.endpoint);
        observability_table["tracing"] = Item::Table(tracing_table);

        doc["observability"] = Item::Table(observability_table);

        // 可选字段
        if let Some(ref user) = config.user {
            doc["user"] = value(user);
        }
        if let Some(ref group) = config.group {
            doc["group"] = value(group);
        }
        if let Some(ref pid) = config.pid {
            doc["pid"] = value(pid);
        }

        // 网络配置
        self.update_bind_config(doc, &config.bind)?;
        self.update_turn_config(doc, &config.turn)?;

        // Supervisor 配置
        if let Some(ref supervisor) = config.supervisor {
            self.update_supervisor_config(doc, supervisor)?;
        }

        Ok(())
    }

    /// 更新绑定配置
    fn update_bind_config(&self, doc: &mut DocumentMut, bind: &BindConfig) -> Result<()> {
        // 确保 bind section 存在
        if !doc.contains_key("bind") {
            doc["bind"] = Item::Table(Table::new());
        }

        // HTTP 配置
        if let Some(ref http) = bind.http {
            let mut http_table = Table::new();
            http_table["domain_name"] = value(&http.domain_name);
            http_table["advertised_ip"] = value(&http.advertised_ip);
            http_table["ip"] = value(&http.ip);
            http_table["port"] = value(http.port as i64);
            doc["bind"]["http"] = Item::Table(http_table);
        }

        // HTTPS 配置
        if let Some(ref https) = bind.https {
            let mut https_table = Table::new();
            https_table["domain_name"] = value(&https.domain_name);
            https_table["advertised_ip"] = value(&https.advertised_ip);
            https_table["ip"] = value(&https.ip);
            https_table["port"] = value(https.port as i64);
            https_table["cert"] = value(&https.cert);
            https_table["key"] = value(&https.key);
            doc["bind"]["https"] = Item::Table(https_table);
        }

        // ICE 配置
        let mut ice_table = Table::new();
        ice_table["domain_name"] = value(&bind.ice.domain_name);
        ice_table["ip"] = value(&bind.ice.ip);
        ice_table["port"] = value(bind.ice.port as i64);
        doc["bind"]["ice"] = Item::Table(ice_table);

        Ok(())
    }

    /// 更新 TURN 配置
    fn update_turn_config(&self, doc: &mut DocumentMut, turn: &TurnConfig) -> Result<()> {
        if !doc.contains_key("turn") {
            doc["turn"] = Item::Table(Table::new());
        }

        doc["turn"]["advertised_ip"] = value(&turn.advertised_ip);
        doc["turn"]["advertised_port"] = value(turn.advertised_port as i64);
        doc["turn"]["relay_port_range"] = value(&turn.relay_port_range);
        doc["turn"]["realm"] = value(&turn.realm);

        Ok(())
    }

    /// 更新 Supervisor 配置
    fn update_supervisor_config(
        &self,
        doc: &mut DocumentMut,
        supervisor: &SupervisorConfig,
    ) -> Result<()> {
        let mut supervisor_table = Table::new();
        supervisor_table["connect_timeout_secs"] = value(supervisor.connect_timeout_secs as i64);
        supervisor_table["status_report_interval_secs"] =
            value(supervisor.status_report_interval_secs as i64);
        supervisor_table["health_check_interval_secs"] =
            value(supervisor.health_check_interval_secs as i64);
        supervisor_table["enable_tls"] = value(supervisor.enable_tls);
        if let Some(ref domain) = supervisor.tls_domain {
            supervisor_table["tls_domain"] = value(domain);
        }
        if let Some(ref cert) = supervisor.client_cert {
            supervisor_table["client_cert"] = value(cert);
        }
        if let Some(ref key) = supervisor.client_key {
            supervisor_table["client_key"] = value(key);
        }
        if let Some(ref ca) = supervisor.ca_cert {
            supervisor_table["ca_cert"] = value(ca);
        }
        let supervisord = &supervisor.supervisord;
        let mut supervisord_table = Table::new();
        supervisord_table["node_name"] = value(&supervisord.node_name);
        supervisord_table["ip"] = value(&supervisord.ip);
        supervisord_table["port"] = value(supervisord.port as i64);
        supervisord_table["advertised_ip"] = value(&supervisord.advertised_ip);
        supervisor_table["supervisord"] = Item::Table(supervisord_table);

        let client = &supervisor.client;
        let mut client_table = Table::new();
        client_table["node_id"] = value(&client.node_id);
        client_table["endpoint"] = value(&client.endpoint);
        client_table["shared_secret"] = value(&client.shared_secret);
        supervisor_table["client"] = Item::Table(client_table);
        if supervisor.max_clock_skew_secs != 300 {
            supervisor_table["max_clock_skew_secs"] = value(supervisor.max_clock_skew_secs as i64);
        }
        doc["supervisor"] = Item::Table(supervisor_table);

        Ok(())
    }

    // 辅助方法
    fn needs_http_services(&self, config: &ActrixConfig) -> bool {
        config.is_signaling_enabled() || config.is_ais_enabled()
    }

    fn needs_supervisor(&self, _config: &ActrixConfig) -> bool {
        Confirm::with_theme(&self.theme)
            .with_prompt("配置 Supervisor 平台集成")
            .default(false)
            .interact()
            .unwrap_or(false)
    }

    fn select_server_host(&self) -> Result<String> {
        let local_ips = NetworkUtils::get_local_ips()?;
        let mut choices: Vec<String> = local_ips
            .iter()
            .map(|ip| format!("{} ({})", ip, self.classify_ip(ip)))
            .collect();

        choices.push("输入自定义 IP/域名".to_string());

        clear_input_buffer();

        let selection = Select::with_theme(&self.theme)
            .with_prompt("选择服务器 IP 或域名")
            .items(&choices)
            .default(0)
            .interact()?;

        if selection < local_ips.len() {
            Ok(local_ips[selection].to_string())
        } else {
            let custom: String = Input::with_theme(&self.theme)
                .with_prompt("输入自定义 IP 或域名")
                .interact_text()?;
            Ok(custom)
        }
    }

    fn classify_ip(&self, ip: &IpAddr) -> &'static str {
        match ip {
            IpAddr::V4(ipv4) => {
                let octets = ipv4.octets();
                if octets[0] == 127 {
                    "本地回环"
                } else if octets[0] == 10
                    || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
                    || (octets[0] == 192 && octets[1] == 168)
                {
                    "私有 IPv4"
                } else {
                    "公网 IPv4"
                }
            }
            IpAddr::V6(_) => "IPv6",
        }
    }

    fn prompt_port(&self, service: &str, default: u16) -> Result<u16> {
        clear_input_buffer();

        loop {
            let input: String = Input::with_theme(&self.theme)
                .with_prompt(format!("{} 端口", service))
                .default(default.to_string())
                .interact_text()?;

            if input == default.to_string() {
                return Ok(default);
            }

            match input.parse::<u16>() {
                Ok(port) if validate_port(port) => return Ok(port),
                _ => println!("❌ 无效端口。请输入 1-65535 之间的端口号。"),
            }
        }
    }

    fn choose_config_location(&self) -> Result<PathBuf> {
        println!("📁 配置文件位置");
        println!("===============");

        clear_input_buffer();

        let default_path = PathBuf::from("/etc/actor-rtc-actrix/config.toml");

        let config_path: String = Input::with_theme(&self.theme)
            .with_prompt("配置文件路径")
            .default(default_path.to_string_lossy().to_string())
            .interact_text()?;

        let path = PathBuf::from(config_path);

        // 检查文件是否已存在
        if path.exists() {
            println!("⚠️  配置文件已存在: {}", path.display());
            let overwrite = Confirm::with_theme(&self.theme)
                .with_prompt("覆盖现有文件？")
                .default(false)
                .interact()?;

            if !overwrite {
                anyhow::bail!("用户取消覆盖现有配置文件");
            }
        }

        println!("✅ 配置文件位置: {}", path.display());
        println!();

        Ok(path)
    }
}
