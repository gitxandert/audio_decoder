use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};

pub mod aiff {
    use super::*;

    fn print_id(vec: &mut Vec<u8>, start: &mut usize, end: &mut usize) {
        end += 4;

        for i in start..end {
            print!("{}", char::from(vec[i]));
        }

        start = end;

        println!("");
    }

    fn parse_bytes(bytes: &mut Vec, start: &mut usize, end: &mut usize, inc: usize) -> io::Result<u32> {
        let mut value: u32 = 0;

        end += inc;

        // big-endian
        let mut shift: u32 = 3;
        for i in start..end {
            let b: u8 = bytes[i]?;

            value += b as u32 << (shift * 8);

            shift -= 1;
        }

        start = end;

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

    // only care about COMM and SSND chunks,
    // so adjust this to search only for those and
    // extract the relevant information
    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

        let mut reader = buf.iter().copied();

        // FORM
        print_id(&mut reader, 4);

        // parse chunks as big-endian
        let be: bool = false;

        let form_size: u32 = parse_bytes(&mut reader, 4, be)?;
        println!("Form size: {form_size}");

        // AIFF
        print_id(&mut reader, 4);
        
        println!("");

        // COMM
        print_id(&mut reader, 4);

        let comm_size: u32 = parse_bytes(&mut reader, 4, be)?;
        if comm_size == 18 {
            println!("Comm size: {comm_size}");
        } else {
            eprintln!("Comm size not 18; que?");
        }

        let num_channels: u32 = parse_bytes(&mut reader, 2, be)?;
        println!("Num channels: {num_channels}");

        let num_frames: u32 = parse_bytes(&mut reader, 4, be)?;
        println!("Num sample frames: {num_frames}");

        let sample_size: u32 = parse_bytes(&mut reader, 2, be)?;
        println!("Sample size: {sample_size}");

        // 80 bit floating-point sample rate
        let mut rate_bytes = [0u8; 10];
        for i in 0..10 {
            rate_bytes[i] = reader.next().unwrap();
        }
        let sample_rate: f64 = parse_ieee_extended(rate_bytes);
        println!("Sample rate: {sample_rate}");
        
        println!("");

        // SSND
        print_id(&mut reader, 4);

        let ssnd_size: u32 = parse_bytes(&mut reader, 4, be)?;
        println!("Data size: {ssnd_size}");

        // typically 0
        let offset: u32 = parse_bytes(&mut reader, 4, be)?;
        println!("Offset: {offset}");
        // also typically 0
        let block_size: u32 = parse_bytes(&mut reader, 4, be)?;
        println!("Block size: {block_size}");

        Ok(Vec::<u8>::new())
    }
} // end pub mod aiff
