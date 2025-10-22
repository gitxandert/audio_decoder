use std::fs::File;
use std::io::{self, Read, BufReader};
use std::ops::{Shl, BitOr, AddAssign};

// WAV file parser
fn main() -> io::Result<()> {
    let f = BufReader::new(File::open("assets/fairies.wav")?);
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

    // "fmt "
    println!("\n");
    for _ in 1..=4 {
        if let Some(Ok(b)) = bytes.next() {
            print!("{}", char::from(b));
        }
    }
    println!("");

    let fmt_size: u32 = parse_bytes(&mut bytes, 4)?;
    println!("Chunk size: {}", fmt_size);

    let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut bytes, 2)?)
    else {
        eprintln!("Error parsing format tag");
        return Ok(())
    };
    println!("Format code: {fmt_tag:?}");

    let n_chan: u32 = parse_bytes(&mut bytes, 2)?;
    println!("Num channels: {n_chan}");

    let sample_rate: u32 = parse_bytes(&mut bytes, 4)?;
    println!("Sample rate: {sample_rate}");

    let data_rate: u32 = parse_bytes(&mut bytes, 4)?;
    println!("Ave bytes /sec: {data_rate}");

    let data_blk_sz: u32 = parse_bytes(&mut bytes, 2)?;
    println!("Block size: {data_blk_sz}");
    
    let bits_per: u32 = parse_bytes(&mut bytes, 2)?;
    println!("Bits per sample: {bits_per}");
   
    if fmt_tag == FormatCode::WaveFormatExtensible && fmt_size >= 40 {
        let cb_size: u32 = parse_bytes(&mut bytes, 2)?;
        println!("Extension size: {cb_size}");
   
        if cb_size > 0 {
            let valid_bits: u32 = parse_bytes(&mut bytes, 2)?;
            println!("Valid bits per sample: {valid_bits}");

            let dw_channel_mask: u32 = parse_bytes(&mut bytes, 4)?;
            println!("Speaker position mask: {dw_channel_mask}");
     
            let old_fmt: u32 = parse_bytes(&mut bytes, 2)?;
            println!("GUID: {old_fmt}");

            // skip over Microsoft stuff
            // TODO: compare against audio media subtype
            for _ in 0..14 {
                if let Some(Ok(b)) = bytes.next() {}
            }
            print!("\n"); 
        }
    }

    Ok(())
}

// helper to parse groups of num bytes
fn parse_bytes<I, T>(bytes: &mut I, num: usize) -> io::Result<T>
where
    I: Iterator<Item = io::Result<u8>>,
    T: From<u8> + Shl<u32, Output = T> + BitOr<Output = T> + AddAssign + Copy + Default,
{
    let mut value = T::default();

    for i in 0..num {
        let b = bytes.next().ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"))??;
        value += T::from(b) << (i as u32 * 8);
    }

    Ok(value)
}

// format codes
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormatCode {
    WaveFormatPcm = 0x0001,
    WaveFormatIeeeFloat = 0x0003,
    WaveFormatAlaw = 0x0006,
    WaveFormatMulaw = 0x0007,
    WaveFormatExtensible = 0xFFFE,
}

impl FormatCode {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::WaveFormatPcm),
            0x0003 => Some(Self::WaveFormatIeeeFloat),
            0x0006 => Some(Self::WaveFormatAlaw),
            0x0007 => Some(Self::WaveFormatMulaw),
            0xFFFE => Some(Self::WaveFormatExtensible),
            _ => None,
        }
    }
}


