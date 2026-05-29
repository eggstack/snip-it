fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/sync.proto");
    tonic_prost_build::configure()
        .build_server(true)
        .out_dir("src")
        .compile_protos(&["proto/sync.proto"], &["proto/"])?;
    Ok(())
}
