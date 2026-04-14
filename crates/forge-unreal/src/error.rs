use thiserror::Error;

/// Result alias for parser operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors raised while parsing a uasset.
#[derive(Error, Debug)]
pub enum Error {
    #[error("data is not a uasset")]
    InvalidFile,
    #[error("asset has unsupported legacy version value {0:?}")]
    UnsupportedVersion(i32),
    #[error("asset has unsupported UE4 version value {0:?}")]
    UnsupportedUE4Version(i32),
    #[error("asset has unsupported UE5 version value {0:?}")]
    UnsupportedUE5Version(i32),
    #[error("asset saved without asset version information")]
    UnversionedAsset,
    #[error("failed to parse data: {0:?}")]
    ParseFailure(binread::Error),
    #[error("failed to read or seek stream: {0:?}")]
    Io(std::io::Error),
    #[error("failed to parse string in asset: {0:?}")]
    InvalidString(std::string::FromUtf8Error),
}

impl From<binread::Error> for Error {
    fn from(e: binread::Error) -> Self {
        Error::ParseFailure(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// Returned when a [`crate::NameReference`] points outside the name table.
#[derive(Error, Debug)]
#[error("invalid name index in asset: {0:?}")]
pub struct InvalidNameIndexError(pub u32);
