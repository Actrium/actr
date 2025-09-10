fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/echo.proto");
    
    tonic_build::configure()
        .protoc_arg("--plugin=protoc-gen-actorframework=/mnt/sdb1/actor-rtc/actr/target/debug/protoc-gen-actorframework")
        .protoc_arg(format!("--actorframework_out={}", std::env::var("OUT_DIR").unwrap()))
        .compile(&["proto/echo.proto"], &["proto/"])?;
    
    Ok(())
}