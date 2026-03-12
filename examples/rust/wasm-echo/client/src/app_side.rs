//! Interactive shell — reads from stdin, sends echo requests via ActrRef

use crate::client_workload::ClientWorkload;
use crate::echo::EchoRequest;
use actr_hyper::ActrRef;
use tracing::{error, info};

pub struct AppSide {
    pub actr_ref: ActrRef<ClientWorkload>,
}

impl AppSide {
    pub async fn run(self) {
        info!("[App] Started");
        println!("===== WASM Echo Client App =====");
        println!("Type messages to send to WASM echo server (type 'quit' to exit):");

        use std::io::Write;
        use tokio::io::{AsyncBufReadExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();

        print!("> ");
        std::io::stdout().flush().unwrap();

        while let Ok(Some(line)) = reader.next_line().await {
            let line = line.trim().to_string();

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
                message: line.clone(),
            };

            info!("[App] Sending to local ClientWorkload: {}", line);

            match self.actr_ref.call(request).await {
                Ok(response) => {
                    println!("\n[Received reply] {}", response.reply);
                }
                Err(e) => {
                    error!("[App] Failed to call: {:?}", e);
                    println!("\n[Error] {}", e);
                }
            }

            print!("> ");
            std::io::stdout().flush().unwrap();
        }

        info!("[App] Shutting down");
    }
}
