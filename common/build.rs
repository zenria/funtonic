fn main() {
    tonic_build::configure()
        .out_dir("src/generated")
        .compile(&["proto/tasks/tasks.proto"], &["proto/tasks"])
        .unwrap();
}
