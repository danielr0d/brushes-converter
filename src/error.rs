use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConverterError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("image decode error: {0}")]
    Image(#[from] image::ImageError),

    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),

    #[error("malformed brush archive: {0}")]
    Malformed(String),

    #[error("unrecognized file format for {0}")]
    UnknownFormat(String),

    #[error("{0} import/export is not implemented yet")]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, ConverterError>;
