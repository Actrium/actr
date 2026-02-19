//! STUN 服务器实现
//!
//! 提供 STUN 协议服务器功能，用于 NAT 发现和网络穿越

pub mod error;

// Re-export error types for convenience
pub use error::{ErrorSeverity, Result, StunError};

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, info};
use webrtc_stun::message::{BINDING_REQUEST, BINDING_SUCCESS, Message};
use webrtc_stun::xoraddr::XorMappedAddress;

/// Create and run a STUN server with graceful shutdown support
pub async fn create_stun_server_with_shutdown(
    socket: Arc<UdpSocket>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    info!(
        "Starting STUN server with shutdown support on {}",
        socket.local_addr()?
    );

    let mut buffer = vec![0u8; 1500]; // Standard MTU size for UDP packets

    loop {
        tokio::select! {
            // Handle incoming UDP packets
            result = socket.recv_from(&mut buffer) => {
                match result {
                    Ok((len, src_addr)) => {
                        let packet_data = &buffer[..len];

                        // Check if this might be a STUN message before processing
                        if is_stun_message(packet_data) {
                            debug!("Received potential STUN packet from {} ({} bytes)", src_addr, len);

                            // Process the packet in the background to avoid blocking the receive loop
                            let socket_clone = socket.clone();
                            let packet_data = packet_data.to_vec();

                            tokio::spawn(async move {
                                if let Err(e) = process_packet(socket_clone, &packet_data, src_addr).await {
                                    error!("Failed to process STUN packet from {}: {}", src_addr, e);
                                }
                            });
                        } else {
                            debug!("Received non-STUN packet from {} ({} bytes), ignoring", src_addr, len);
                        }
                    }
                    Err(e) => {
                        error!("Error receiving UDP packet: {}", e);
                        return Err(e.into());
                    }
                }
            }

            // Handle shutdown signal
            _ = shutdown_rx.recv() => {
                info!("Received shutdown signal, stopping STUN server");
                break;
            }
        }
    }

    info!("STUN server has been shut down");
    Ok(())
}

/// Checks if the given data could be a STUN message.
/// STUN messages (and TURN messages that are not ChannelData) have the first two bits as 00.
pub fn is_stun_message(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    (data[0] & 0xC0) == 0
}

/// Processes a potential STUN packet.
/// If it's a BINDING_REQUEST, it sends a BINDING_SUCCESS response.
/// Other STUN message types are ignored.
pub async fn process_packet(socket: Arc<UdpSocket>, data: &[u8], src: SocketAddr) -> Result<()> {
    let mut msg = Message::new();
    // The `write` method decodes a message from a byte slice.
    if let Err(e) = msg.write(data) {
        // This might not be a STUN message or it's malformed.
        // Log as trace because other handlers (e.g., TURN) might process it.
        debug!(
            "Failed to parse data as STUN message from {}: {}, data_len={}",
            src,
            e,
            data.len()
        );
        return Ok(());
    }

    if msg.typ == BINDING_REQUEST {
        if let Err(e) = handle_binding_request(&socket, &msg, src).await {
            error!("Failed to handle STUN binding request from {}: {}", src, e);
            // Even if handling fails, we don't want to kill the server loop, so return Ok.
        }
    } else {
        debug!(
            "Received non-binding STUN message type {:?} from {}",
            msg.typ, src
        );
    }

    Ok(())
}

async fn handle_binding_request(
    socket: &UdpSocket,
    request: &Message,
    src: SocketAddr,
) -> Result<()> {
    debug!("Processing binding request from {}", src);

    // Create Binding Success response
    let mut response_msg = Message::new();
    response_msg.set_type(BINDING_SUCCESS);
    response_msg.transaction_id = request.transaction_id;

    // Add XOR-MAPPED-ADDRESS attribute
    let xor_addr = XorMappedAddress {
        ip: src.ip(),
        port: src.port(),
    };

    // Use build to correctly assemble the message with attributes
    response_msg.build(&[Box::new(xor_addr)])?;

    // Send response
    socket.send_to(&response_msg.raw, src).await?;
    debug!("Sent STUN Binding Success response to {}", src);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    // use webrtc_stun::client::ClientBuilder; // Unused in current tests
    use anyhow::Result;
    // Make process_packet and handle_binding_request available
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::UdpSocket;
    use tokio::time::timeout;
    use webrtc_stun::agent::TransactionId;
    use webrtc_stun::message::{BINDING_REQUEST, Getter as _};
    use webrtc_stun::xoraddr::XorMappedAddress;

    #[tokio::test]
    async fn test_stun_packet_processing_works() -> Result<()> {
        // Setup a "server" socket that our process_packet will use to send responses
        let server_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let server_addr = server_socket.local_addr()?;

        // Setup a "client" socket
        let client_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let client_addr = client_socket.local_addr()?;
        client_socket.connect(server_addr).await?; // Client sends to server_addr

        // Client: Craft a STUN Binding Request message
        let mut request_msg = Message::new();
        request_msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])?;

        // Client: Send the request to the server socket
        client_socket.send(&request_msg.raw).await?;

        // Server side: receive the packet and process it
        let mut recv_buf = [0; 1024];
        let (len, src_addr) = timeout(
            Duration::from_secs(1),
            server_socket.recv_from(&mut recv_buf),
        )
        .await??;
        assert_eq!(src_addr, client_addr);

        // Call our STUN packet processor
        process_packet(server_socket.clone(), &recv_buf[..len], src_addr).await?;

        // Client: Wait for the response
        let (response_len, _) = timeout(
            Duration::from_secs(1),
            client_socket.recv_from(&mut recv_buf),
        )
        .await??;

        let mut response_stun_msg = Message::new();
        response_stun_msg.write(&recv_buf[..response_len])?;

        // Verify the response
        assert_eq!(response_stun_msg.typ, BINDING_SUCCESS);
        assert_eq!(response_stun_msg.transaction_id, request_msg.transaction_id);

        let mut xor_addr = XorMappedAddress::default();
        xor_addr.get_from(&response_stun_msg)?;

        assert_eq!(xor_addr.ip, client_addr.ip());
        assert_eq!(xor_addr.port, client_addr.port());

        Ok(())
    }

    #[tokio::test]
    async fn test_is_stun_message() {
        // Valid STUN Binding Request (first two bits 00)
        let mut msg = Message::new();
        msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])
            .unwrap();
        assert!(is_stun_message(&msg.raw));

        // Not a STUN message (e.g., first two bits 01 - ChannelData)
        let not_stun_data = [0x40, 0x01, 0x00, 0x00]; // Example ChannelData prefix
        assert!(!is_stun_message(&not_stun_data));

        // Empty data
        assert!(!is_stun_message(&[]));

        // Too short but looks like STUN
        // is_stun_message only checks the first byte, full parsing is in process_packet
        let short_stun_data = [0x00, 0x01];
        assert!(is_stun_message(&short_stun_data));
    }

    #[tokio::test]
    async fn test_stun_server_with_shutdown() -> Result<()> {
        use std::time::Duration;
        use tokio::sync::broadcast;
        use tokio::time::timeout;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // Setup server socket
        let server_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let server_addr = server_socket.local_addr()?;

        // Start the STUN server in background
        let server_socket_clone = server_socket.clone();
        let server_handle = tokio::spawn(async move {
            create_stun_server_with_shutdown(server_socket_clone, shutdown_rx).await
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Setup client socket and send request
        let client_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        client_socket.connect(server_addr).await?;

        // Create and send STUN binding request
        let mut request_msg = Message::new();
        request_msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])?;
        client_socket.send(&request_msg.raw).await?;

        // Receive response
        let mut recv_buf = [0; 1024];
        let (response_len, _) = timeout(
            Duration::from_secs(1),
            client_socket.recv_from(&mut recv_buf),
        )
        .await??;

        // Parse and verify response
        let mut response_stun_msg = Message::new();
        response_stun_msg.write(&recv_buf[..response_len])?;
        assert_eq!(response_stun_msg.typ, BINDING_SUCCESS);

        // Test shutdown
        let _ = shutdown_tx.send(());

        // Wait for server to shut down
        let _ = timeout(Duration::from_secs(1), server_handle).await??;

        Ok(())
    }
}
