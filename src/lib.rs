pub mod mpeg;
pub mod aiff;
pub mod wav;
pub mod decode_helpers;
pub mod playback;

use decode_helpers::{DecodeResult, DecodeError, AudioFile};
use playback::run_gart;

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

        run_gart(vec![af], af.sample_rate, af.num_channels);
    }

    #[test]
    fn test_aiff() {
        let path = "assets/winterly.aif";

        let af = match aiff::parse(path) {
            Ok(file) => file,
            Err(error) => panic!("{:?}", error),
        };

        run_gart(vec![af], af.sample_rate, af.num_channels);
    }
}
