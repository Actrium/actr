use tracing::{Instrument as _, error, info};

use crate::client_workload::ClientWorkload;
use crate::generated::echo::EchoRequest;
use actr_runtime::prelude::*;

pub struct AppSide {
    pub actr_ref: ActrRef<ClientWorkload>,
}

impl AppSide {
    pub async fn run(self) {
        info!("[App] started");
        println!("===== WS Echo Client App =====");
        println!("messagewillvia/through WebSocket direct connectionchannelsend[...]server");
        println!("[...]messagesend（[...] 'quit' [...]）：");

        use tokio::io::{AsyncBufReadExt, BufReader, stdin};
        let stdin = stdin();
        let mut reader = BufReader::new(stdin).lines();

        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().unwrap();

        while let Ok(Some(line)) = reader.next_line().await {
            let line = line.trim();

            if line == "quit" || line == "exit" {
                info!("[App] [...]request[...]");
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

            info!("[App] sendmessage: {}", line);
            let span = tracing::info_span!("app_side_call", line = line);
            match self.actr_ref.call(request).instrument(span).await {
                Ok(response) => {
                    println!("\n[[...]] {}", response.reply);
                }
                Err(e) => {
                    error!("[App] [...]failed: {:?}", e);
                    println!("\n[error] {}", e);
                }
            }

            print!("> ");
            std::io::stdout().flush().unwrap();
        }

        info!("[App] [...]close/shutdown");
    }
}
