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

    static BITRATES: [[u32; 5]; 14] = [
        [32,	32,	  32,	  32,	  8],
        [64,	48,	  40,	  48,	  16],
        [96,	56,	  48,	  56,	  24],
        [128,	64,	  56,	  64,	  32],
        [160,	80,	  64,	  80,	  40],
        [192,	96,	  80,	  96,	  48],
        [224,	112,	96,	  112,	56],
        [256,	128,	112,	128,	64],
        [288,	160,	128,	144,	80],
        [320,	192,	160,	160,	96],
        [352,	224,	192,	176,	112],
        [384,	256,	224,	192,	128],
        [416,	320,	256,	224,	144],
        [448,	384,	320,	256,	160],
    ];

    fn match_bitrate(row: u8, V: &u8, L: &u8) -> u32 {
        let VL = (V << 2) & L;
        let col = match VL {
            0xF => 0,
            0xE => 1,
            0xD => 2,
            0xB => 3,
            _   => 4,
        };

        BITRATES[row][col]
    }

    fn match_sr(bits: &u8, v_id: &u8) -> f64 {
        let base: f64 = match v_id {
            0x3 => 32000f64,
            0x2 => 16000f64,
            0x0 => 8000f64,
            _   => 0f64,
        };

        let FF = bits >> 2;
        let sr: f64 = match FF {
            0x0 => base * 1.378125,
            0x1 => base * 1.5,
            0x2 => base,
            _   => 0f64,
        }

        sr
    }
 
    fn parse_header<I>(it: &mut I)
    where I: Iterator<Item = u8> {
        println!("Parsing header:");
        // the following is parsed by bits:
        // AAA
        // (23-21) = guaranteed set at this point
        //
        // B B
        // (20,19) = audio version ID
        // bit 20 will only ever *not* be set for MPEG v2.5
        let Some(AAAB) = it.next();
        let v_id: u8 = (AAAB & 0x1) << 1;
        //
        // bit 19 is 0 for MPEG V2 or 1 for MPEG V1
        //
        let Some(BCCD) = it.next();
        v_id |= BCCD & 0x1;

        // CC
        // (18,17) = layer description
        // 01 - Layer III
        // 10 - Layer II
        // 11 - Layer I
        let layer: u8 = (BCCD >> 1) & 0x3;

        // D
        // (16) = protection bit
        // if 0, check for 16bit CRC after header
        let not_protected: bool = BCCD & 0x1;
        
        // EEEE
        // (15,12) = bitrate index
        // this depends on combinations of version (V) and layer (L)
        // apply V2 to V2.5
        // 0000 and 1111 are not allowed
        let Some(EEEE) = it.next();
        if EEEE == 0 || EEEE == 0xF {
            return io::Error::new(io::ErrorKind::Unsupported, "This application does not support 'free' or 'bad' bitrates");
        }
        let bitrate: u32 = match_bitrate(EEEE, &v_id, &layer);

        // FF
        // (11,10) = sampling rate
        // varies by V
        let Some(FFGH) = it.next();
        let sr: f64 = match_sr(&FFGH, &v_id);
        
        // G
        // (9) = padding bit
        let padded: bool = (FFGH >> 1) & 0x1;

        // H
        // (8) = private bit
        // ignore
        // 
        // I
        // (7,6) = channel mode
        let Some(IIJJ) = it.next();
        let channel_mode = IIJJ >> 2;
        
        // J
        // (5,4) = mode extension (only if channel_mode = joint stereo)
        let mode_ext = IIJJ & 0xC;

        // bits 3-0 are not pertinent
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

    pub fn parse(path: &str) -> io::Result<()> {
        let buf = buf_file(path)?;
        let mut buf_iter = buf.iter().copied();
 
        // read byes little-endian
        let le: bool = true;     

        // RIFF
        print_id(&mut buf_iter, 4);

        let riff_size: u32 = parse_bytes(&mut buf_iter, 4, le)?;
        println!("Chunk size: {riff_size}");

        // WAVE
        print_id(&mut buf_iter, 4);

        println!("");

        // "fmt "
        print_id(&mut buf_iter, 4);        

        let fmt_size: u32 = parse_bytes(&mut buf_iter, 4, le)?;
        println!("Chunk size: {}", fmt_size);

        let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut buf_iter, 2, le)?)
        else {
            eprintln!("Error parsing format tag");
            return Ok(())
        };
        println!("Format code: {fmt_tag:?}");

        let n_chan: u32 = parse_bytes(&mut buf_iter, 2, le)?;
        println!("Num channels: {n_chan}");

        let sample_rate: u32 = parse_bytes(&mut buf_iter, 4, le)?;
        println!("Sample rate: {sample_rate}");

        let data_rate: u32 = parse_bytes(&mut buf_iter, 4, le)?;
        println!("Ave bytes /sec: {data_rate}");

        let data_blk_sz: u32 = parse_bytes(&mut buf_iter, 2, le)?;
        println!("Block size: {data_blk_sz}");

        let bits_per: u32 = parse_bytes(&mut buf_iter, 2, le)?;
        println!("Bits per sample: {bits_per}");

        // if !WaveFormatPcm (i.e. is extensible)
        if fmt_size >= 18 {
            // extension is either 0 or 22
            let cb_size: u32 = parse_bytes(&mut buf_iter, 2, le)?;
            println!("Extension size: {cb_size}");

            if cb_size > 0 {
                let valid_bits: u32 = parse_bytes(&mut buf_iter, 2, le)?;
                println!("Valid bits per sample: {valid_bits}");

                let dw_channel_mask: u32 = parse_bytes(&mut buf_iter, 4, le)?;
                println!("Speaker position mask: {dw_channel_mask}");

                let old_fmt: u32 = parse_bytes(&mut buf_iter, 2, le)?;
                println!("GUID: {old_fmt}");

                // skip over Microsoft stuff
                // TODO: compare against audio media subtype
                for _ in 0..14 {
                    if let Some(b) = buf_iter.next() {}
                }
                print!("\n");
            }
        }
        
        Ok(())
    }
} // end pub mod wav
