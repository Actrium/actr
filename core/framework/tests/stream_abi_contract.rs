use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn wasm_workload_wit_declares_data_stream_surface() {
    let wit = fs::read_to_string(manifest_dir().join("wit/actr-workload.wit"))
        .expect("read workload WIT");

    for expected in [
        "record data-stream",
        "variant payload-type",
        "register-stream: func(",
        "unregister-stream: func(",
        "send-data-stream: func(",
        "on-data-stream: func(",
    ] {
        assert!(
            wit.contains(expected),
            "actr-workload.wit should declare `{expected}` for wasm data stream support"
        );
    }
}

#[test]
fn dynclib_abi_declares_data_stream_surface() {
    let abi = fs::read_to_string(manifest_dir().join("src/guest/dynclib_abi.rs"))
        .expect("read dynclib ABI");

    for expected in [
        "HOST_REGISTER_STREAM",
        "HOST_UNREGISTER_STREAM",
        "HOST_SEND_DATA_STREAM",
        "GUEST_DATA_STREAM",
        "HostRegisterStreamV1",
        "HostUnregisterStreamV1",
        "HostSendDataStreamV1",
        "GuestDataChunkV1",
    ] {
        assert!(
            abi.contains(expected),
            "dynclib_abi.rs should declare `{expected}` for dynclib data stream support"
        );
    }
}
