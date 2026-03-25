fn main() -> Result<(), Box<dyn std::error::Error>> {
    prost_build::compile_protos(&["../../proto/echo.proto"], &["../../proto/"])?;
    Ok(())
}
