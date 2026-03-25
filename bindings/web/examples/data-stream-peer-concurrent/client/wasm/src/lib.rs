mod stream_client_service;

use std::rc::Rc;
use wasm_bindgen::prelude::*;

pub use actr_runtime_sw::*;

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());

    log::info!("[DataStreamClient] WASM initialized");
}

#[wasm_bindgen]
pub fn register_stream_client_handler() {
    log::info!("[DataStreamClient] Registering stream client workload");

    actr_runtime_sw::register_workload(actr_runtime_sw::WasmWorkload::new(Rc::new(
        |route_key, bytes, ctx| {
            let route_key = route_key.to_string();
            let bytes = bytes.to_vec();
            Box::pin(async move {
                let (service, method) = if let Some(last_dot) = route_key.rfind('.') {
                    (&route_key[..last_dot], &route_key[last_dot + 1..])
                } else {
                    (route_key.as_str(), "")
                };

                match service {
                    "data_stream.StreamClient" => {
                        stream_client_service::handle_request(method, &bytes, ctx).await
                    }
                    _ => Err(format!("Unknown service: {}", service)),
                }
            })
        },
    )));
}
