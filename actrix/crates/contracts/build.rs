fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Compile all proto files
    // - common.proto: shared types for supervisor.v1
    // - supervisor.proto: SupervisorService (Node calls Supervisor)
    // - supervised.proto: SupervisedService (Supervisor calls Node)
    // - keyserver.proto: KeyServer service (imports common.proto)
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/common.proto",
                "proto/supervisor.proto",
                "proto/supervised.proto",
                "proto/keyserver.proto",
            ],
            &["proto/"],
        )?;

    // Rebuild if any proto file changes
    println!("cargo:rerun-if-changed=proto/common.proto");
    println!("cargo:rerun-if-changed=proto/supervisor.proto");
    println!("cargo:rerun-if-changed=proto/supervised.proto");
    println!("cargo:rerun-if-changed=proto/keyserver.proto");

    Ok(())
}
