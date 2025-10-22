use std::fs::File;
use std::io::{self, Read, SeekFrom, prelude::*};
use std::fmt::UpperHex;

use audio_decoder::{parse_bytes, wav};

fn main() -> io::Result<()> {
    let mut f = File::open("assets/lazy_beat.mp3")?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let mut buf_iter = buf.iter().copied().peekable();

    while let Some(b) = buf_iter.next() {
        if b == 0xFF {
            if let Some(&next) = buf_iter.peek() {
                if next & 0xE0 == 0xE0 {
                    parse_header(&mut buf_iter);
                }
            }
        }
    }

    Ok(())
}

fn parse_header<I>(it: &mut I)
where I: Iterator<Item = u8>, {
    println!("Parsing header:");
    for _ in 0..3 {
        println!("{:#X}", it.next().unwrap());
    }
    println!("");
}
