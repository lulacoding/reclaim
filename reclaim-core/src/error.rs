//! Crate-wide error type.

/// Errors produced by `reclaim-core` and shared across the workspace.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("failed to parse TOML: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("failed to serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("protected path, refusing: {0}")]
    ProtectedPath(String),
}

/// Convenience alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_converts_via_from() {
        fn read_missing() -> Result<String> {
            std::fs::read_to_string("this-file-does-not-exist-anywhere.toml")?;
            unreachable!()
        }
        let err = read_missing().unwrap_err();
        assert!(matches!(err, Error::Io(_)));
    }

    #[test]
    fn protected_path_error_message_contains_the_path() {
        let err = Error::ProtectedPath("C:\\Windows".to_string());
        assert!(err.to_string().contains("C:\\Windows"));
    }

    #[test]
    fn toml_parse_error_converts_via_from() {
        fn parse_bad() -> Result<toml::Value> {
            Ok(toml::from_str("not = [valid")?)
        }
        let err = parse_bad().unwrap_err();
        assert!(matches!(err, Error::TomlParse(_)));
    }
}
