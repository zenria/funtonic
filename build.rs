fn main() {
    tonic_build::compile_protos("proto/helloworld/helloworld.proto").unwrap();
    //tonic_build::compile_protos("proto/tasks/tasks.proto").unwrap();
    tonic_build::configure()
        .out_dir("src/generated")
        .compile(&["proto/tasks/tasks.proto"], &["proto/tasks"])
        .unwrap();
}
