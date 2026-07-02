use super::*;

#[test]
fn compile_rejects_non_wasm_bytes() {
    let err = WasmHost::compile(b"definitely not a wasm component").unwrap_err();
    assert!(matches!(err, WasmError::LoadFailed(_)));
    assert!(err.to_string().contains("Component"));
}

#[test]
fn compile_rejects_empty_bytes() {
    let err = WasmHost::compile(&[]).unwrap_err();
    assert!(matches!(err, WasmError::LoadFailed(_)));
}

#[test]
fn compile_rejects_legacy_core_module_magic() {
    // `\0asm` magic + invalid body must still fail (host requires
    // Component Model binaries).
    let bogus = b"\0asm\x01\x00\x00\x00garbage";
    let err = WasmHost::compile(bogus).unwrap_err();
    assert!(matches!(err, WasmError::LoadFailed(_)));
}
