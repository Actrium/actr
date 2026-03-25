fn main() {
    prost_build::Config::new()
        .compile_protos(&["../proto/client.proto"], &["../proto/"])
        .expect("Failed to compile client.proto");
}
