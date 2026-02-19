fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile all proto files
    // - common.proto: shared types for admin.v1
    // - admin.proto: ControlService (Node calls Admin)
    // - node_admin.proto: NodeAdminService (Admin calls Node)
    // - keyserver.proto: KeyServer service (imports common.proto)
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/common.proto",
                "proto/admin.proto",
                "proto/node_admin.proto",
                "proto/keyserver.proto",
            ],
            &["proto/"],
        )?;

    // Rebuild if any proto file changes
    println!("cargo:rerun-if-changed=proto/common.proto");
    println!("cargo:rerun-if-changed=proto/admin.proto");
    println!("cargo:rerun-if-changed=proto/node_admin.proto");
    println!("cargo:rerun-if-changed=proto/keyserver.proto");

    Ok(())
}
