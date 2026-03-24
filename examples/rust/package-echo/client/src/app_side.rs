//! Interactive shell — reads from stdin and calls the remote echo server directly.

use crate::echo::{EchoRequest, EchoResponse};
use actr_hyper::ActrRef;
use actr_protocol::ActrId;
use tracing::{error, info};

pub struct AppSide {
    pub actr_ref: ActrRef,
    pub server_id: ActrId,
}

impl AppSide {
    pub async fn run(self) {
        info!("[App] Started");
        println!("===== Package Echo Client App =====");
        println!("Type messages to send to the echo server (type 'quit' to exit):");

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

            info!("[App] Sending to remote echo server: {}", line);

            match self
                .actr_ref
                .call_remote(self.server_id.clone(), request)
                .await
            {
                Ok(response) => {
                    let response: EchoResponse = response;
                    println!("\n[Received reply] {}", response.reply);
                }
                Err(e) => {
                    error!("[App] Failed to call remote: {:?}", e);
                    println!("\n[Error] {}", e);
                }
            }

            print!("> ");
            std::io::stdout().flush().unwrap();
        }

        info!("[App] Shutting down");
    }
}
