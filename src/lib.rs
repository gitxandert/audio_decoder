pub mod mpeg;
pub mod aiff;
pub mod wav;
pub mod decode_helpers;

use decode_helpers::{DecodeResult, DecodeError, AudioFile};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav() {
        let path = "assets/fairies.wav";

        match wav::parse(path) {
            Ok(file) => println!("{:?}", file),
            Err(error) => eprintln!("{:?}", error),
        };
    }

    #[test]
    fn test_aiff() {
        let path = "assets/winterly.aif";

        match aiff::parse(path) {
            Ok(file) => println!("{:?}", file),
            Err(error) => eprintln!("{:?}", error),
        };
    }
}
