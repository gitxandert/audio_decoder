use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};
use crate::decode_helpers::{DecodeResult, DecodeError};

pub mod aiff {
    use super::*;

    fn print_id(vec: &mut Vec<u8>, start: &mut usize, end: &mut usize) -> DecodeResult<()> {
        *end += 4;

        for i in *start..*end {
            let c = match vec.get(i) {
                Some(val) => val,
                None => return Err(DecodeError::UnexpectedEof),
            };

            print!("{}", char::from(*c));
        }

        *start = *end;

        println!("");

        Ok(())
    }

    fn parse_bytes(bytes: &mut Vec<u8>, start: &mut usize, end: &mut usize, inc: usize) -> DecodeResult<u32> {
        let mut value: u32 = 0;

        *end += inc;

        // big-endian
        let mut shift: u32 = 24;
        for i in *start..*end {
            let b: u8 = match bytes.get(i) {
                Some(val) => *val,
                None => return Err(DecodeError::UnexpectedEof),
            };

            value += (b as u32) << shift;

            shift -= 8;
        }

        *start = *end;

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
    pub fn parse(path: &str) -> DecodeResult<Vec<u8>> {
        let mut f = File::open(path)?;
        let mut reader = Vec::new();
        f.read_to_end(&mut reader)?;

        let mut start = 0;
        let mut end = 0;

        // FORM
        print_id(&mut reader, &mut start, &mut end)?;

        let form_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Form size: {form_size}");

        // AIFF
        print_id(&mut reader, &mut start, &mut end)?;
        
        println!("");

        // COMM
        print_id(&mut reader, &mut start, &mut end)?;

        let comm_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        if comm_size == 18 {
            println!("Comm size: {comm_size}");
        } else {
            eprintln!("Comm size not 18; que?");
        }

        let num_channels: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
        println!("Num channels: {num_channels}");

        let num_frames: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Num sample frames: {num_frames}");

        let sample_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
        println!("Sample size: {sample_size}");

        // 80 bit floating-point sample rate
        let mut rate_bytes = [0u8; 10];
        end += 10;
        let mut i = 0;
        for _ in start..end {
            rate_bytes[i] = reader[i];
            i += 1;
        }
        let sample_rate: f64 = parse_ieee_extended(rate_bytes);
        println!("Sample rate: {sample_rate}");
        
        println!("");

        // SSND
        print_id(&mut reader, &mut start, &mut end)?;

        let ssnd_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Data size: {ssnd_size}");

        // typically 0
        let offset: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Offset: {offset}");
        // also typically 0
        let block_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Block size: {block_size}");

        Ok(Vec::<u8>::new())
    }
} // end pub mod aiff
