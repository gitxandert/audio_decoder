use std::io;

use audio_decoder::{wav, mpeg};

fn main() -> io::Result<()> {
    mpeg::parse("assets/lazy_beat.mp3")?;

    Ok(())
}
