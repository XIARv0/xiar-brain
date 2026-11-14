// MODIFIED by XIAR: minimal stand-in for `tonic::Status`, which upstream used purely as an
// error type in gRPC-conversion code. Removing `tonic` drops the whole gRPC/TLS/HTTP stack
// (and its C-based `ring` dependency) from the build.
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    message: String,
}

impl Status {
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid argument: {}", self.message)
    }
}

impl std::error::Error for Status {}
