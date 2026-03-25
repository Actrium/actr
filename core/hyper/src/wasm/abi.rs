//! WASM Host-Guest ABI definition
//!
//! This ABI is designed to be **runtime-agnostic**, compatible with Wasmtime (desktop/server)
//! and WAMR (mobile/embedded), using only WASM core spec (linear memory + function calls),
//! without depending on the Component Model.
//!
//! # Guest must implement these exported functions
//!
//! ```text
//! /// Initialize the actor (receives prost-encoded InitPayloadV1)
//! actr_init(init_ptr: i32, init_len: i32) -> i32
//!
//! /// Handle one runtime frame (prost-encoded AbiFrame)
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

/// Convert an ABI error code to a human-readable description.
///
/// Error codes are defined in [`actr_framework::guest::abi::code`].
pub fn describe_error_code(code: i32) -> &'static str {
    use actr_framework::guest::abi::code;
    match code {
        code::SUCCESS => "success",
        code::GENERIC_ERROR => "generic error",
        code::INIT_FAILED => "initialization failed",
        code::HANDLE_FAILED => "message handling failed",
        code::ALLOC_FAILED => "memory allocation failed",
        code::PROTOCOL_ERROR => "protocol error (malformed message)",
        code::BUFFER_TOO_SMALL => "reply buffer too small",
        code::UNSUPPORTED_OP => "unsupported operation code",
        _ => "unknown error",
    }
}
