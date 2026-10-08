//! Generates the worker protocol from `proto/` with protox, so no `protoc` is needed.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_root = "../../proto";
    println!("cargo:rerun-if-changed={proto_root}");
    let descriptors = protox::compile(["igloo/worker/v1/worker.proto"], [proto_root])?;
    tonic_prost_build::configure()
        .build_transport(false)
        .compile_fds(descriptors)?;
    Ok(())
}
