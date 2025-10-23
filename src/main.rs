use std::io;

use audio_decoder::{mpeg, aiff, wav};

fn main() -> Result<(), ParseError<'static>> {
    let path = "assets/winterly.aif";
    let ext: &str = match path.rsplit_once(|b: char| b == '.') {
        Some((before, after)) if !before.is_empty() && !after.is_empty() => after,
        _ => "",
    };

    match ext {
        "mp3" => mpeg::parse(path),
        "wav" => wav::parse(path),
        "aif" => aiff::parse(path),
        _ => return Err(ParseError::UnsupportedFormat(ext)),
    }?;

    Ok(())
}

#[allow(dead_code)]
#[derive(Debug)]
enum ParseError<'a> {
    UnsupportedFormat(&'a str),
    Io(io::Error),
}

impl From<io::Error> for ParseError<'_> {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
