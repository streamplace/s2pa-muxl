use std::io;

/// Errors returned by muxl operations.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Mp4(mp4::Error),
    InvalidMp4(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Mp4(e) => write!(f, "MP4 error: {e}"),
            Error::InvalidMp4(msg) => write!(f, "invalid MP4: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<mp4::Error> for Error {
    fn from(e: mp4::Error) -> Self {
        Error::Mp4(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
