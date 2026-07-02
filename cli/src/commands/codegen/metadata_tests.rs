use super::*;
use tempfile::TempDir;

fn svc_model(name: &str, side: super::super::ProtoSide) -> ServiceModel {
    ServiceModel {
        name: name.into(),
        package: format!("pkg.{name}"),
        proto_file: PathBuf::from(format!("protos/{name}.proto")),
        relative_path: PathBuf::from(format!("protos/{name}.proto")),
        side,
        methods: vec![MethodModel {
            name: "Echo".into(),
            snake_name: "echo".into(),
            input_type: "EchoRequest".into(),
            output_type: "EchoResponse".into(),
            route_key: "echo.EchoService.Echo".into(),
        }],
        actr_type: Some("acme:Echo:1.0.0".into()),
    }
}

#[test]
fn from_proto_model_populates_local_and_remote() {
    let model = ProtoModel {
        files: vec![],
        local_services: vec![svc_model("EchoService", super::super::ProtoSide::Local)],
        remote_services: vec![],
    };
    let meta = ActrGenMetadata::from_proto_model(SupportedLanguage::Rust, &model);
    assert_eq!(meta.local_services.len(), 1);
    assert_eq!(meta.local_services[0].name, "EchoService");
    assert_eq!(meta.language, "rust");
    assert!(meta.remote_services.is_empty());
}

#[test]
fn metadata_path_joins_output_dir_with_filename() {
    let p = metadata_path(std::path::Path::new("out"));
    assert_eq!(p, std::path::Path::new("out").join(ACTR_GEN_META_FILE));
}

#[test]
fn load_metadata_returns_none_when_file_absent() {
    let dir = TempDir::new().unwrap();
    assert!(load_metadata(dir.path()).unwrap().is_none());
}

#[test]
fn write_and_load_metadata_roundtrip() {
    let dir = TempDir::new().unwrap();
    let meta = ActrGenMetadata {
        plugin_version: "actr-cli".into(),
        language: "rust".into(),
        local_services: vec![],
        remote_services: vec![],
    };
    let path = write_metadata(dir.path(), &meta).unwrap();
    assert!(path.exists());
    let loaded = load_metadata(dir.path()).unwrap().unwrap();
    assert_eq!(loaded.plugin_version, "actr-cli");
}
