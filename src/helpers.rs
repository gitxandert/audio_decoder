use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};

// helpers for AIFF and WAV
//
// parse groups of num bytes
//
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

