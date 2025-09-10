fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/echo.proto");
    let plugin_path = format!(
        "{}/../../target/debug/protoc-gen-actorframework",
        std::env::var("CARGO_MANIFEST_DIR")?
    );
    tonic_build::configure()
        .protoc_arg(format!(
            "--plugin=protoc-gen-actorframework={}",
            plugin_path
        ))
        .protoc_arg(format!(
            "--actorframework_out={}",
            std::env::var("OUT_DIR").unwrap()
        ))
        .compile(&["proto/echo.proto"], &["proto/"])?;

    Ok(())
}
