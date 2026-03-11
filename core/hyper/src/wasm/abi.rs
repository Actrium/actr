//! WASM Host-Guest ABI definition
//!
//! This ABI is designed to be **runtime-agnostic**, compatible with Wasmtime (desktop/server)
//! and WAMR (mobile/embedded), using only WASM core spec (linear memory + function calls),
//! without depending on the Component Model.
//!
//! # Guest must implement these exported functions
//!
//! ```text
//! /// Initialize the actor (receives JSON-encoded config, see WasmActorConfig)
//! actr_init(config_ptr: i32, config_len: i32) -> i32
//!
//! /// Handle one request message (protobuf-encoded RpcEnvelope)
//! /// - req_ptr / req_len: start address and length of request data in linear memory
//! /// - resp_ptr_out: Host-allocated i32 pointer where WASM writes the response data address
//! /// - resp_len_out: Host-allocated i32 pointer where WASM writes the response data length
//! /// Host calls actr_free(resp_ptr, resp_len) after reading the response to free WASM-allocated memory
//! actr_handle(req_ptr: i32, req_len: i32,
//!             resp_ptr_out: i32, resp_len_out: i32) -> i32
//!
//! /// Allocate memory in WASM linear memory (called by Host before writing data)
//! /// Returns the allocated pointer, 0 indicates allocation failure
//! actr_alloc(size: i32) -> i32
//!
//! /// Free WASM linear memory (called by Host after completing read/write)
//! actr_free(ptr: i32, size: i32)
//! ```
//!
//! # Data write protocol (Host -> WASM)
//!
//! ```text
//! 1. host calls actr_alloc(size) -> ptr
//! 2. host writes data to wasm_memory[ptr..ptr+size]
//! 3. host calls target function with ptr, size
//! 4. host calls actr_free(ptr, size) to free memory
//! ```
//!
//! # Data read protocol (WASM -> Host)
//!
//! ```text
//! 1. host allocates two i32 variables on its stack: resp_ptr_out, resp_len_out
//! 2. host writes their addresses to WASM memory via actr_alloc temporary area,
//!    and passes the addresses to actr_handle
//! 3. WASM allocates response memory inside actr_handle, writes ptr/len to resp_ptr_out/resp_len_out
//! 4. host reads resp_ptr_out/resp_len_out, retrieves response data from WASM memory
//! 5. host calls actr_free(resp_ptr, resp_len) to free WASM response memory
//! ```
//!
//! # Error codes (all functions returning i32)
//!
//! | Value | Meaning                         |
//! |-------|---------------------------------|
//! | 0     | Success                         |
//! | -1    | Generic error                   |
//! | -2    | Initialization failed           |
//! | -3    | Message handling failed          |
//! | -4    | Memory allocation failed         |
//! | -5    | Protocol error (malformed message) |

/// WASM exported function names
pub const EXPORT_INIT: &str = "actr_init";
pub const EXPORT_HANDLE: &str = "actr_handle";
pub const EXPORT_ALLOC: &str = "actr_alloc";
pub const EXPORT_FREE: &str = "actr_free";
pub const EXPORT_MEMORY: &str = "memory";

/// ABI error codes
pub mod code {
    pub const SUCCESS: i32 = 0;
    pub const GENERIC_ERROR: i32 = -1;
    pub const INIT_FAILED: i32 = -2;
    pub const HANDLE_FAILED: i32 = -3;
    pub const ALLOC_FAILED: i32 = -4;
    pub const PROTOCOL_ERROR: i32 = -5;
}

/// Convert an ABI error code to a human-readable description
pub fn describe_error_code(code: i32) -> &'static str {
    match code {
        self::code::SUCCESS => "success",
        self::code::GENERIC_ERROR => "generic error",
        self::code::INIT_FAILED => "initialization failed",
        self::code::HANDLE_FAILED => "message handling failed",
        self::code::ALLOC_FAILED => "memory allocation failed",
        self::code::PROTOCOL_ERROR => "protocol error (malformed message)",
        _ => "unknown error",
    }
}

/// WasmActorConfig - JSON structure passed to actr_init during initialization
///
/// The guest side parses this and initializes its internal state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WasmActorConfig {
    /// Actor type (manufacturer:name:version)
    pub actr_type: String,

    /// AID credential (protobuf bytes, base64-encoded)
    pub credential_b64: String,

    /// Actor ID (protobuf bytes, base64-encoded)
    pub actor_id_b64: String,

    /// Realm ID
    pub realm_id: u32,
}
