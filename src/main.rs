use std::io;
use audio_decoder::{
    mpeg, aiff, wav,
    decode_helpers::{DecodeError, DecodeResult}
};

fn main() -> DecodeResult<()> {
    let path = "assets/fairies.wav";
    let ext: &str = match path.rsplit_once(|b: char| b == '.') {
        Some((before, after)) if !before.is_empty() && !after.is_empty() => after,
        _ => "",
    };

    // TODO: figure out actual mpeg decoding...
    /*match ext {
        "mp3" => mpeg::parse(path),
        "wav" => wav::parse(path),
        "aif" => aiff::parse(path),
        _ => return Err(DecodeError::UnsupportedFormat(String::from(ext))),
    }?;
    */

    Ok(())
}
