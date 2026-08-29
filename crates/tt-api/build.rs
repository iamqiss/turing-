fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = &[
        "../../proto/common/v1.proto",
        "../../proto/auth/v1.proto",
        "../../proto/messaging/v1.proto",
    ];
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(protos, &["../../proto"])?;
    Ok(())
}
