use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const START_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
use serial_test::serial;

fn choose_port() -> u16 {
    if let Some(p) = std::env::var("ACTRIX_TEST_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return p;
    }
    48080 + (std::process::id() as u16 % 1000)
}

fn write_minimal_config(dir: &PathBuf, port: u16) -> PathBuf {
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).expect("create data dir");
    let config_path = dir.join("config.toml");
    let mut f = fs::File::create(&config_path).expect("create config file");
    writeln!(
        f,
        r#"
name = "actrix-test"
enable = 16  # ENABLE_KS
env = "dev"
sqlite_path = "{sqlite}"
actrix_shared_key = "0123456789abcdef0123456789abcdef"
location_tag = "local,test,default"

[bind]
[bind.http]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = {port}

[bind.ice]
domain_name = "localhost"
advertised_ip = "127.0.0.1"
ip = "127.0.0.1"
port = 0

[turn]
advertised_ip = "127.0.0.1"
advertised_port = 3478
relay_port_range = "49152-65535"
realm = "actor-rtc.local"

[services.ks]
# defaults

[observability.log]
output = "console"
level = "info"

[process]
pid = "{pid}"
"#,
        sqlite = data_dir.display(),
        port = port,
        pid = dir.join("actrix.pid").display()
    )
    .expect("write config");
    config_path
}

fn spawn_actrix(config: &PathBuf, log_path: &PathBuf) -> Child {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_actrix"));
    let log_file = fs::File::create(log_path).expect("create log file");
    Command::new(bin)
        .arg("--config")
        .arg(config)
        .stdout(Stdio::from(log_file.try_clone().expect("dup log")))
        .stderr(Stdio::from(log_file))
        .spawn()
        .expect("spawn actrix")
}

async fn wait_for_health(url: &str, child: &mut Child, log_path: &PathBuf) {
    let client = reqwest::Client::new();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap_or(None) {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            panic!("actrix exited early: status={status:?}\nlogs:\n{log}");
        }

        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        if start.elapsed() > START_TIMEOUT {
            let log = fs::read_to_string(log_path).unwrap_or_default();
            panic!("health check not ready at {}\nlogs:\n{}", url, log);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn graceful_shutdown(mut child: Child) {
    // Try SIGINT first (Unix only); fallback to kill.
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if start.elapsed() > SHUTDOWN_TIMEOUT {
                    let _ = child.kill();
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return,
        }
    }
}

#[tokio::test]
#[serial]
async fn actrix_starts_serves_health_and_shuts_down() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let port = choose_port();
    let config_path = write_minimal_config(&tmp.path().to_path_buf(), port);
    let log_path = tmp.path().join("actrix.log");
    let mut child = spawn_actrix(&config_path, &log_path);

    let health_url = format!("http://127.0.0.1:{}/ks/health", port);
    wait_for_health(&health_url, &mut child, &log_path).await;

    let resp = reqwest::get(&health_url).await.expect("health request");
    assert!(resp.status().is_success());
    let body: Value = resp.json().await.expect("health json");
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["service"], "ks");

    graceful_shutdown(child);
}
