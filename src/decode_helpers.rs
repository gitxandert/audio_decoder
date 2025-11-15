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

#[derive(Debug)]
pub struct AudioFile {
    pub file_name: String,
    pub format: String,
    pub sample_rate: u32,
    pub num_channels: u32,
    pub bits_per_sample: u32,
    pub samples: Vec<i16>,
}

impl AudioFile {
    pub fn new(file_name: &str, format: &str, sample_rate: u32, num_channels: u32, bits_per_sample: u32, samples: Vec<i16>) -> Self {
        Self {
            file_name: file_name.to_string(),
            format: format.to_string(),
            sample_rate,
            num_channels,
            bits_per_sample,
            samples
        }
    }
}
