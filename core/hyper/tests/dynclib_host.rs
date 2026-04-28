//! DynclibHost loader-level tests.
//!
//! Covers loader failure modes that don't need a real .so image.  The
//! happy-path (load → instantiate → dispatch) is exercised by
//! `dynclib_actor_e2e.rs`, which serializes execution against the
//! process-global guest state through `DYNCLIB_SERIAL`.

#![cfg(feature = "dynclib-engine")]

use actr_hyper::dynclib::{DynclibError, DynclibHost};

/// Loading a non-existent library path should return LoadFailed
#[test]
fn test_load_nonexistent_library() {
    let result = DynclibHost::load("/tmp/nonexistent_library_xyz.so");
    assert!(result.is_err(), "loading non-existent library should fail");
    let err = result.unwrap_err();
    assert!(
        matches!(err, DynclibError::LoadFailed(_)),
        "error should be LoadFailed, got: {err:?}"
    );
}
