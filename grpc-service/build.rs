fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("protoc path: {}", protobuf_src::protoc().to_string_lossy());
    std::env::set_var("PROTOC", protobuf_src::protoc());
    tonic_prost_build::configure()
        .out_dir("src/")
        .compile_protos(&["proto/tasks/tasks.proto"], &["proto/tasks"])?;
    Ok(())
}
