use gart::{
    mpeg, aiff, wav,
    decode_helpers::{DecodeError, DecodeResult},
    playback::play_file,
};

fn main() -> DecodeResult<()> {
    let path = "assets/fairies.wav";
    /*let ext: &str = match path.rsplit_once(|b: char| b == '.') {
        Some((before, after)) if !before.is_empty() && !after.is_empty() => after,
        _ => "",
    };

    // TODO: figure out actual mpeg decoding...
    match ext {
        "mp3" => mpeg::parse(path),
        "wav" => wav::parse(path),
        "aif" => aiff::parse(path),
        _ => return Err(DecodeError::UnsupportedFormat(String::from(ext))),
    }?;
    */

    let af = match wav::parse(path) {
        Ok(file) => file,
        Err(error) => panic!("Error with file"),
    };

    play_file(af);

    Ok(())
}
