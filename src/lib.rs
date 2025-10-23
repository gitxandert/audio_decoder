use std::fs::File;
use std::io::{self, Read};
use std::ops::{Shl, BitOr, AddAssign};

// helper to buffer files
// (since this is done for all three supported types)
fn buf_file(path: &str) -> io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    
    Ok(buf)
}

pub mod mpeg {
    use super::*;

    // literally just look for headers,
    // then suck out the data
    pub fn parse(path: &str) -> io::Result<()> {
        let buf = buf_file(path)?;
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
}// end pub mod mpeg

// helpers for AIFF and WAV
//
// parse groups of num bytes
pub fn parse_bytes<I, T>(bytes: &mut I, num: usize, le: bool) -> io::Result<T>
where
    I: Iterator<Item = u8>,
    T: From<u8> + Shl<u32, Output = T> + BitOr<Output = T> + AddAssign + Copy + Default,
{
    let mut value = T::default();

    for i in 0..num {
        let b = bytes.next().ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "EOF"))?;
        
        if le {
            // little-endian
            value += T::from(b) << (i as u32 * 8);
        } else {
            // big-endian
            value += T::from(b) << ((num - 1 - i) as u32 * 8);
        }
    }

    Ok(value)
}
//
// special function to parse IEEE 80-bit extended floating-point
fn parse_ieee_extended(bytes: [u8; 10]) -> f64 {
    let sign = (bytes[0] & 0x80) != 0;
    let exp = (((bytes[0] & 0x7F) as u16) << 8) | bytes[1] as u16;

    // 64-bit mantissa (explicit integer bit at bit 63)
    let mut mant: u64 = 0;
    for &b in &bytes[2..] {
        mant = (mant << 8) | b as u64;
    }

    // Zero
    if exp == 0 && mant == 0 {
        return 0.0;
    }

    // Inf/NaN
    if exp == 0x7FFF {
        return if mant == 0 {
            if sign { f64::NEG_INFINITY } else { f64::INFINITY }
        } else {
            f64::NAN
        };
    }

    // value = mantissa * 2^(exp - 16383 - 63)
    let e = (exp as i32) - 16383 - 63;
    let mut val = (mant as f64) * 2f64.powi(e);
    if sign { val = -val; }
    
    val
}
//
// (dev) print ids
// TODO: return &str (to retain info from specific IDs)
fn print_id<I>(iter: &mut I, num: usize)
where I: Iterator<Item = u8> {
    for _ in 0..num {
        if let Some(b) = iter.next() {
            print!("{}", char::from(b));
        }
    }
    println!("");
}

pub mod aiff {
    use super::*;

    // only care about COMM and SSND chunks,
    // so adjust this to search only for those and
    // extract the relevant information
    pub fn parse(path: &str) -> io::Result<()> {
        let buf = buf_file(path)?;
        let mut buf_iter = buf.iter().copied();

        // FORM
        print_id(&mut buf_iter, 4);

        // parse chunks as big-endian
        let be: bool = false;

        let form_size: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        println!("Form size: {form_size}");

        // AIFF
        print_id(&mut buf_iter, 4);
        
        println!("");

        // COMM
        print_id(&mut buf_iter, 4);

        let comm_size: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        if comm_size == 18 {
            println!("Comm size: {comm_size}");
        } else {
            eprintln!("Comm size not 18; que?");
        }

        let num_channels: u32 = parse_bytes(&mut buf_iter, 2, be)?;
        println!("Num channels: {num_channels}");

        let num_frames: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        println!("Num sample frames: {num_frames}");

        let sample_size: u32 = parse_bytes(&mut buf_iter, 2, be)?;
        println!("Sample size: {sample_size}");

        // 80 bit floating-point sample rate
        let mut rate_bytes = [0u8; 10];
        for i in 0..10 {
            rate_bytes[i] = buf_iter.next().unwrap();
        }
        let sample_rate: f64 = parse_ieee_extended(rate_bytes);
        println!("Sample rate: {sample_rate}");
        
        println!("");

        // SSND
        print_id(&mut buf_iter, 4);

        let ssnd_size: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        println!("Data size: {ssnd_size}");

        // typically 0
        let offset: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        println!("Offset: {offset}");
        // also typically 0
        let block_size: u32 = parse_bytes(&mut buf_iter, 4, be)?;
        println!("Block size: {block_size}");

        Ok(())
    }
} // end pub mod aiff

pub mod wav {
    use super::*;

    // format codes
    #[repr(u16)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FormatCode {
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

    // a little more complex than the others
    pub fn parse(path: &str) -> io::Result<()> {
        let buf = buf_file(path)?;
        let mut _buf_iter = buf.iter();
        /* TODO: fix this to parse similarly as in mod mpeg
        // "fmt "
        println!("\n");
        for _ in 1..=4 {
            if let Some(Ok(b)) = bytes.next() {
                print!("{}", char::from(b));
            }
        }
        println!("");
        
        let le: bool = true;

        let fmt_size: u32 = parse_bytes(&mut bytes, 4, le)?;
        println!("Chunk size: {}", fmt_size);

        let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut bytes, 2, le)?)
        else {
            eprintln!("Error parsing format tag");
            return Ok(())
        };
        println!("Format code: {fmt_tag:?}");

        let n_chan: u32 = parse_bytes(&mut bytes, 2, le)?;
        println!("Num channels: {n_chan}");

        let sample_rate: u32 = parse_bytes(&mut bytes, 4, le)?;
        println!("Sample rate: {sample_rate}");

        let data_rate: u32 = parse_bytes(&mut bytes, 4, le)?;
        println!("Ave bytes /sec: {data_rate}");

        let data_blk_sz: u32 = parse_bytes(&mut bytes, 2, le)?;
        println!("Block size: {data_blk_sz}");

        let bits_per: u32 = parse_bytes(&mut bytes, 2, le)?;
        println!("Bits per sample: {bits_per}");

        // if !WaveFormatPcm (i.e. is extensible)
        if fmt_size >= 18 {
            // extension is either 0 or 22
            let cb_size: u32 = parse_bytes(&mut bytes, 2, le)?;
            println!("Extension size: {cb_size}");

            if cb_size > 0 {
                let valid_bits: u32 = parse_bytes(&mut bytes, 2, le)?;
                println!("Valid bits per sample: {valid_bits}");

                let dw_channel_mask: u32 = parse_bytes(&mut bytes, 4, le)?;
                println!("Speaker position mask: {dw_channel_mask}");

                let old_fmt: u32 = parse_bytes(&mut bytes, 2, le)?;
                println!("GUID: {old_fmt}");

                // skip over Microsoft stuff
                // TODO: compare against audio media subtype
                for _ in 0..14 {
                    if let Some(Ok(b)) = bytes.next() {}
                }
                print!("\n");
            }
        }
        */
        Ok(())
    }
} // end pub mod wav
