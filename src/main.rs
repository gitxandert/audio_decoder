use std::fs::File;
use std::io::{self, Read, BufReader};

use audio_decoder::{parse_bytes, wav};

// WAV file parser
fn main() -> io::Result<()> {
    let f = BufReader::new(File::open("assets/winterly.aif")?);
    let mut bytes = f.bytes();

    // RIFF
    for _ in 1..=4 {
        if let Some(Ok(b)) = bytes.next() {
            print!("{}", char::from(b));
        }
    }
    println!("");

    let chunk_size: u32 = parse_bytes(&mut bytes, 4)?;
    println!("Chunk size: {}", chunk_size);

    // WAVE
    for _ in 1..=4 {
        if let Some(Ok(b)) = bytes.next() {
            print!("{}", char::from(b));
        }
    }
    println!("");
    
    // wav::parse(bytes);

    Ok(())
}
