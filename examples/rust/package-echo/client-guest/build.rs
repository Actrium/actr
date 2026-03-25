fn main() {
    prost_build::Config::new()
        .compile_protos(
            &["../proto/echo.proto", "../proto/client.proto"],
            &["../proto/"],
        )
        .expect("Failed to compile protos");
}
