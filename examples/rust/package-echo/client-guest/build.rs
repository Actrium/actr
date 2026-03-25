fn main() {
    prost_build::Config::new()
        .compile_protos(&["../proto/echo.proto"], &["../proto/"])
        .expect("Failed to compile echo.proto");
}
