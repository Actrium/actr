// Guest-side workload: compiled to a WASM Component targeting wasip2.
//
// Imports host: call-raw + log-message.
// Exports workload: on-start, dispatch, on-peer-event, report-error.

wit_bindgen::generate!({
    world: "spike-guest",
    path: "../wit",
    generate_all,
});

use exports::actr::spike::workload::Guest;
use actr::spike::host::{log_message, call_raw};
use actr::spike::types::{ActrId, SpikeError, RpcEnvelope, PeerEvent, ErrorEvent, ErrorCategory};

struct EchoWorkload;

impl Guest for EchoWorkload {
    fn on_start() -> Result<(), SpikeError> {
        log_message("info", "workload starting");

        // Exercise host import returning a result: call-raw
        let target = ActrId { serial_number: 42 };
        match call_raw(target, "probe.Ping", b"hello-from-guest") {
            Ok(reply) => {
                log_message(
                    "debug",
                    &format!(
                        "call-raw ok: {}",
                        String::from_utf8_lossy(&reply)
                    ),
                );
            }
            Err(e) => {
                log_message("warn", &format!("call-raw err: {:?}", e));
            }
        }

        Ok(())
    }

    fn dispatch(env: RpcEnvelope) -> Result<Vec<u8>, SpikeError> {
        // Special route for panic test (Q12)
        if env.route_key == "panic.Panic" {
            panic!("intentional panic in dispatch for trap-propagation test");
        }

        if env.route_key == "bad.Payload" {
            return Err(SpikeError::BadPayload("rejected by guest".to_string()));
        }

        log_message(
            "debug",
            &format!(
                "dispatch route={} req_id={} payload_len={}",
                env.route_key,
                env.request_id,
                env.payload.len()
            ),
        );

        let mut reply = b"echo: ".to_vec();
        reply.extend_from_slice(&env.payload);
        Ok(reply)
    }

    fn on_peer_event(evt: PeerEvent) -> Result<(), SpikeError> {
        log_message(
            "info",
            &format!(
                "peer-event peer_id={} relayed={} ts={}",
                evt.peer.peer_id, evt.peer.relayed, evt.timestamp_ms
            ),
        );
        Ok(())
    }

    fn report_error(evt: ErrorEvent) -> Result<(), SpikeError> {
        let cat_str = match &evt.category {
            ErrorCategory::Network => "network".to_string(),
            ErrorCategory::Protocol => "protocol".to_string(),
            ErrorCategory::Workload => "workload".to_string(),
            ErrorCategory::Other(s) => format!("other({s})"),
        };
        log_message(
            "error",
            &format!(
                "error src={} cat={} ctx_entries={}",
                evt.source,
                cat_str,
                evt.context.len()
            ),
        );
        Ok(())
    }
}

export!(EchoWorkload);
