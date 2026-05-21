//! Integration smoke tests for full web code generation.
//!
//! Requires `protoc` to be available on the PATH. Tests that cannot locate
//! the binary report as skipped instead of hard-failing, to keep minimal
//! environments usable.

use std::path::PathBuf;

use actr_web_protoc_codegen::{WebCodegen, WebCodegenConfig};

fn protoc_available() -> bool {
    std::process::Command::new("protoc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn reset_generated_dir(path: &std::path::Path) {
    if path.exists() {
        std::fs::remove_dir_all(path).expect("remove stale generated directory");
    }
    std::fs::create_dir_all(path).expect("create generated directory");
}

#[test]
fn generates_rust_typescript_actor_ref_and_react_hook() {
    if !protoc_available() {
        eprintln!("skipping: protoc not on PATH");
        return;
    }

    let manifest_dir = manifest_dir();
    let fixture_dir = manifest_dir.join("tests/fixtures");
    let proto_path = fixture_dir.join("user_service.proto");
    let rust_out = manifest_dir.join("tests/generated-rust");
    let ts_out = manifest_dir.join("tests/generated-ts");

    reset_generated_dir(&rust_out);
    reset_generated_dir(&ts_out);

    let config = WebCodegenConfig::builder()
        .proto_file(proto_path)
        .rust_output(&rust_out)
        .ts_output(&ts_out)
        .include(&fixture_dir)
        .with_react_hooks(true)
        .with_formatting(false)
        .build()
        .expect("build config");

    let files = WebCodegen::new(config).generate().expect("generate files");

    assert_eq!(files.rust_files.len(), 2);
    assert_eq!(files.ts_types.len(), 2);
    assert_eq!(files.ts_actor_refs.len(), 1);
    assert_eq!(files.react_hooks.len(), 1);
    assert_eq!(files.total_count(), 6);

    let rust_actor = std::fs::read_to_string(rust_out.join("user_service.rs"))
        .expect("read generated rust actor");
    assert!(rust_actor.contains("pub struct UserServiceActor"));
    assert!(rust_actor.contains("pub age: Option<i32>"));
    assert!(rust_actor.contains("pub tags: Vec<String>"));
    assert!(rust_actor.contains("pub async fn list_users"));

    let types_ts = std::fs::read_to_string(ts_out.join("user-service.types.ts"))
        .expect("read generated TS types");
    assert!(types_ts.contains("export interface CreateUserRequest"));
    assert!(types_ts.contains("age?: number;"));
    assert!(types_ts.contains("tags: string[];"));
    assert!(types_ts.contains("export function encodeGetUserRequest"));
    assert!(types_ts.contains("export function decodeUserEvent"));

    let actor_ref_ts = std::fs::read_to_string(ts_out.join("user-service.actor-ref.ts"))
        .expect("read generated actor-ref");
    assert!(actor_ref_ts.contains("export class UserServiceActorRef extends ActorRef"));
    assert!(actor_ref_ts.contains("async getUser(request: GetUserRequest)"));
    assert!(actor_ref_ts.contains("this.callRaw('UserService:GetUser'"));
    assert!(actor_ref_ts.contains("subscribeListUsers(callback: (data: User) => void)"));
    assert!(actor_ref_ts.contains("subscribeWatchUsers(callback: (data: UserEvent) => void)"));

    let hook_ts = std::fs::read_to_string(ts_out.join("use-user-service.ts"))
        .expect("read generated React hook");
    assert!(hook_ts.contains("export function useUserService(actorId: string)"));
    assert!(hook_ts.contains("const getUser = useCallback("));
    assert!(hook_ts.contains("return {"));
}
