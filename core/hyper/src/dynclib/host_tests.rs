use super::*;

#[test]
fn load_missing_file_errors() {
    // A path that does not exist must surface LoadFailed, not panic.
    let err = DynclibHost::load("/nonexistent/actor.dylib").unwrap_err();
    assert!(matches!(err, DynclibError::LoadFailed(_)));
}

#[test]
fn load_non_library_file_errors() {
    // A real file that is not a shared library must also fail to load.
    let tmp = std::env::temp_dir().join("actr-not-a-lib.txt");
    std::fs::write(&tmp, b"not a shared library").unwrap();
    let err = DynclibHost::load(&tmp).unwrap_err();
    // Either LoadFailed (dlopen) or MissingSymbol (if the platform somehow
    // opens it). Both are expected non-panic failures.
    let _ = err;
    let _ = std::fs::remove_file(&tmp);
}
