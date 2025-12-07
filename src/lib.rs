pub mod audio_processing;
pub mod file_parsing;

use audio_processing::runtime::run_blast;
use file_parsing::decode_helpers::{DecodeResult, DecodeError, AudioFile};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav() {
        println!("parsing a wav file");
        let path = "assets/fairies.wav";

        let af = match wav::parse(path) {
            Ok(file) => file,
            Err(error) => panic!("Error with file"),
        };

        run_blast(vec![af], af.sample_rate, af.num_channels);
    }

    #[test]
    fn test_aiff() {
        let path = "assets/winterly.aif";

        let af = match aiff::parse(path) {
            Ok(file) => file,
            Err(error) => panic!("{:?}", error),
        };

        run_blast(vec![af], af.sample_rate, af.num_channels);
    }
}
