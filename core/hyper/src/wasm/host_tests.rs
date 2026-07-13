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

/// Compile a WAT component source into a real wasmtime [`Component`] on an
/// engine configured exactly like production ([`build_engine`]).
fn probe_wat(src: &str) -> WasmResult<WasmWorkloadKind> {
    let bytes = wat::parse_str(src).expect("test WAT must assemble");
    let engine =
        build_engine(&crate::config::WasmRuntimeLimits::default()).expect("engine must build");
    let component = Component::from_binary(&engine, &bytes).expect("component must load");
    probe_world(&component, &engine)
}

#[test]
fn probe_world_rejects_component_without_workload_world() {
    // `(false, false)` arm: a component that exports no recognised
    // `actr:workload/workload` world (here: no exports at all) must be a
    // clean `LoadFailed`, never a panic.
    let err = probe_wat("(component)").unwrap_err();
    assert!(matches!(err, WasmError::LoadFailed(_)));
    assert!(
        err.to_string().contains("no recognised"),
        "unexpected message: {err}"
    );
}

#[test]
fn probe_world_rejects_component_exporting_both_worlds() {
    // `(true, true)` arm: a component that exports both the 0.1.0 and 0.2.0
    // worlds is ambiguous and must be a clean `LoadFailed`, never a panic.
    let src = r#"
        (component
          (instance $v1)
          (instance $v2)
          (export "actr:workload/workload@0.1.0" (instance $v1))
          (export "actr:workload/workload@0.2.0" (instance $v2))
        )
    "#;
    let err = probe_wat(src).unwrap_err();
    assert!(matches!(err, WasmError::LoadFailed(_)));
    assert!(
        err.to_string().contains("both"),
        "unexpected message: {err}"
    );
}
