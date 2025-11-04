#[derive(Debug)]
pub enum DecodeError {
    Io(std::io::Error),
    UnsupportedFormat(String),
    UnexpectedEof,
    InvalidData(String),
}

pub type DecodeResult<T> = Result<T, DecodeError>;

impl From<std::io::Error> for DecodeError {
    fn from(err: std::io::Error) -> Self {
        DecodeError::Io(err)
    }
}
