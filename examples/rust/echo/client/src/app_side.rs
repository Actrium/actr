use tracing::{error, info};

use crate::client_workload::ClientWorkload;
use crate::generated::echo::EchoRequest;
use actr_hyper::prelude::*;

pub struct AppSide {
    pub actr_ref: ActrRef<ClientWorkload>,
}

impl AppSide {
    pub async fn run(self) {
        info!("[App] Started");

        use tokio::io::{AsyncBufReadExt, BufReader, stdin};
        let stdin = stdin();
        let mut reader = BufReader::new(stdin).lines();

        print!("> ");
        use std::io::Write;
        std::io::stdout().flush().unwrap();

        while let Ok(Some(line)) = reader.next_line().await {
            let line = line.trim();

            if line == "quit" || line == "exit" {
                info!("[App] User requested exit");
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

            info!("[App] Sending: {}", line);
            match self.actr_ref.call(request).await {
                Ok(response) => {
                    println!("\n[Received reply] {}", response.reply);
                }
                Err(e) => {
                    error!("[App] Call failed: {:?}", e);
                    println!("\n[Error] {}", e);
                }
            }

            print!("> ");
            std::io::stdout().flush().unwrap();
        }

        info!("[App] Shutting down");
    }
}
