use super::ServerConfig;

pub fn run(config: ServerConfig) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!(
            "server mode is supported on Windows and macOS; requested bind address was {}",
            config.bind
        ),
    ))
}
