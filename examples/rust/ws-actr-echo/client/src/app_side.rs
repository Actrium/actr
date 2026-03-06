use tracing::{Instrument as _, error, info};

use crate::client_workload::ClientWorkload;
use crate::generated::echo::EchoRequest;
use actr_runtime::prelude::*;

pub struct AppSide {
    pub actr_ref: ActrRef<ClientWorkload>,
}

impl AppSide {
    pub async fn run(self) {
        info!("[App] 已启动");
        println!("===== WS Echo Client App =====");
        println!("消息将通过 WebSocket 直连通道发送到服务端");
        println!("输入消息发送（输入 'quit' 退出）：");

        use tokio::io::{AsyncBufReadExt, BufReader, stdin};
        let stdin = stdin();
        let mut reader = BufReader::new(stdin).lines();

        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().unwrap();

        while let Ok(Some(line)) = reader.next_line().await {
            let line = line.trim();

            if line == "quit" || line == "exit" {
                info!("[App] 用户请求退出");
                break;
            }

            if line.is_empty() {
                print!("> ");
                std::io::stdout().flush().unwrap();
                continue;
            }

            let request = EchoRequest {
                message: line.to_string(),
            };

            info!("[App] 发送消息: {}", line);
            let span = tracing::info_span!("app_side_call", line = line);
            match self.actr_ref.call(request).instrument(span).await {
                Ok(response) => {
                    println!("\n[收到回复] {}", response.reply);
                }
                Err(e) => {
                    error!("[App] 调用失败: {:?}", e);
                    println!("\n[错误] {}", e);
                }
            }

            print!("> ");
            std::io::stdout().flush().unwrap();
        }

        info!("[App] 正在关闭");
    }
}
