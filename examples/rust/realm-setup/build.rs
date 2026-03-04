use std::io::Result;
use std::path::PathBuf;

fn find_proto_dir(pkg_name: &str) -> Option<PathBuf> {
    // Use cargo_metadata to find the package manifest path, then assume proto/ lives next to it
    let meta = match cargo_metadata::MetadataCommand::new().exec() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("failed to run cargo metadata: {}", e);
            return None;
        }
    };

    for pkg in meta.packages {
        if pkg.name == pkg_name {
            let manifest = PathBuf::from(pkg.manifest_path);
            if let Some(dir) = manifest.parent() {
                return Some(dir.join("proto"));
            }
        }
    }
    None
}

fn main() -> Result<()> {
    // Locate proto directory via Cargo dependency (actrix-proto)
    let actrix_proto_dir = find_proto_dir("actrix-proto").expect(
        "failed to locate actrix-proto crate via cargo metadata; ensure actrix-proto is a dependency",
    );

    // Files we want to compile (relative to the located proto dirs)
    let supervised = actrix_proto_dir.join("supervised.proto");

    // Convert to owned Strings so slices remain valid for the duration of the call
    let proto_owned: Vec<String> = vec![supervised.to_string_lossy().into_owned()];
    let proto_files: Vec<&str> = proto_owned.iter().map(|s| s.as_str()).collect();

    let includes_owned: Vec<String> = vec![actrix_proto_dir.to_string_lossy().into_owned()];
    let include_dirs: Vec<&str> = includes_owned.iter().map(|s| s.as_str()).collect();

    // Use OUT_DIR for generated files (standard Cargo build directory)
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");

    // Compile protobuf files for gRPC client only
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .out_dir(&out_dir)
        .compile_protos(proto_files.as_slice(), include_dirs.as_slice())?;

    Ok(())
}
