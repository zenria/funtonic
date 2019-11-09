fn main() {
    tonic_build::configure()
        .out_dir("src/")
        .compile(&["proto/tasks/tasks.proto"], &["proto/tasks"])
        .unwrap();
}
