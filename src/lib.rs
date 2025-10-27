use std::fs::File;
use std::io::{self, Read, SeekFrom};
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

    // iterate through frames by frame size
    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let buf = buf_file(path)?;
        let mut reader = buf.iter().copied().peekable();
        let mut data: Vec<u8> = Vec::new();
        loop {
            if let Some(b) = reader.next() {
                if b == 0xFF {
                    if let Some(&next) = reader.peek() {
                        if next & 0xE0 == 0xE0 {
                            let header = parse_header(&mut reader)?;
                            let frame_len = compute_frame_len(header)?;
                            for _ in 0..frame_len {
                                if let Some(b) = reader.next() {
                                    data.push(b);
                                } else {
                                    break;
                                }
                            }
                        } else {
                            continue;
                        }
                    } else {
                        break;
                    }
                } else {
                    continue;
                }
            } else {
                break;
            }
        }
    
        Ok(data)
    }

    #[derive(Debug)]
    struct Header {
       version: f32,
       layer: i32,
       protected: bool,
       bitrate: u32,
       sr: f64,
       padded: bool,
       channel_mode: u8,
    }

    impl Header {
        fn format(version: u8, layer: u8, not_protected: u8, bitrate: u32, sr: f64, padded: u8, channel_mode: u8) -> Self {
            let version: f32 = match version {
                0x0 => 2.5f32,
                0x2 => 2.0f32,
                0x3 => 1.0f32,
                _   => 0.0f32, // check if greater than 0
            };

            let layer: i32 = match layer {
                0x1 => 3,
                0x2 => 2,
                0x3 => 1,
                _   => 0, // check if greater than 0
            };

            let protected: bool = match not_protected {
                0 => true,
                _ => false,
            };

            let padded: bool = match padded {
                1 => true,
                _ => false,
            };

            Self {
                version,
                layer,
                protected,
                bitrate,
                sr,
                padded,
                channel_mode
            }
        }

        fn barf(&self) -> (f32, i32, bool, u32, f64, bool, u8) {
                (self.version, self.layer, self.protected, self.bitrate, self.sr, self.padded, self.channel_mode)
        }
    }

    // returns frame size in bytes
    fn compute_frame_len(header: Header) -> io::Result<u32> {
        let (_, layer, _, br, sr, _, _) = header.barf();
        
        let br: f64 = br as f64 * 1000f64;
        let frame_len: f64 = match layer {
            3 => 144f64 * br as f64 / sr,
            2 => 144f64 * br as f64 / sr,
            1 => (12f64 * br as f64 / sr) * 4f64,
            _ => {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "Cannot parse reserved layer"))
            }
        };

        // subtract the header
        Ok(frame_len as u32 - 4)
    }

    static BITRATES: [[u32; 5]; 16] = [
        [0,   0,    0,    0,    0], // dummy
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
        [0,   0,    0,    0,    0], // dummy
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

        BITRATES[row as usize][col]
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
            0x0 => base * 1.378125f64,
            0x1 => base * 1.5f64,
            0x2 => base,
            _   => 0f64,
        };

        sr
    }

    static mut header_count: u32 = 1;

    fn parse_header<I>(it: &mut I) -> io::Result<Header>
    where I: Iterator<Item = u8> {
        let unex_eof = io::Error::new(io::ErrorKind::UnexpectedEof, "EOF");
        unsafe {
            let count: *const u32 = &raw const header_count;
            println!("Parsing header {}", *count);
        }
        
        let Some(AAAB_BCCD) = it.next() else { return Err(unex_eof) };
        // AAA
        // (23-21) = guaranteed set at this point
        //
        // B B
        // (20,19) = audio version ID
        // bit 20 will only ever *not* be set for MPEG v2.5
        let AAAB = AAAB_BCCD >> 4;
        let mut version: u8 = (AAAB & 0x1) << 1;
        //
        // bit 19 is 0 for MPEG V2 or 1 for MPEG V1
        //
        let BCCD = AAAB_BCCD & 0x0F;
        version |= BCCD & 0x1;

        print!("MPEG Version ");
        match version {
            0x0 => print!("2.5\n"),
            0x1 => {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "Unsupported audio version"))
            },
            0x2 => print!("2\n"),
            0x3 => print!("1\n"),
            _   => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Did you parse the version id correctly?"))
            },
        };

        // CC
        // (18,17) = layer description
        // 01 - Layer III
        // 10 - Layer II
        // 11 - Layer I
        let layer: u8 = (BCCD >> 1) & 0x3;
        
        print!("Layer ");
        match layer {
            0x0 => {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "Cannot parse reserved layer"))
            },
            0x1 => print!("III\n"),
            0x2 => print!("II\n"),
            0x3 => print!("I\n"),
            _   => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Did you parse the version id correctly?"))
            },
        };

        // D
        // (16) = protection bit
        // if 0, check for 16bit CRC after header
        let not_protected: u8 = BCCD & 0x1;
        if not_protected == 1{
            println!("Not protected");
        } else {
            println!("Protected");
        }
        
        let Some(EEEE_FFGH) = it.next() else { return Err(unex_eof) };
        // EEEE
        // (15,12) = bitrate index
        // this depends on combinations of version (V) and layer (L)
        // apply V2 to V2.5
        // 0000 and 1111 are not allowed
        let EEEE = EEEE_FFGH >> 4;
        let mut bitrate: u32;
        if EEEE == 0 || EEEE == 0xF {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "This application does not support 'free' or 'bad' bitrates"));
        } else {
            bitrate = match_bitrate(EEEE, &version, &layer);
            println!("Bitrate: {bitrate}");
        }

        // FF
        // (11,10) = sampling rate
        // varies by V
        let FFGH = EEEE_FFGH & 0x0F;
        let sr: f64 = match_sr(&FFGH, &version);
        println!("Sample rate: {sr}");

        // G
        // (9) = padding bit
        let padded: u8 = (FFGH >> 1) & 0x1;
        if padded == 1 {
            println!("Padded");
        } else {
            println!("Not padded");
        }

        // H
        // (8) = private bit
        // ignore
        //
        let Some(IIJJ_KLMM) = it.next() else { return Err(unex_eof) };
        // I
        // (7,6) = channel mode
        let IIJJ = IIJJ_KLMM >> 4;
        let channel_mode = IIJJ >> 2;
        match channel_mode {
            0x0 => println!("Stereo"),
            0x1 => println!("Joint stereo"),
            0x2 => println!("Dual channel (stereo)"),
            0x3 => println!("Single channel (mono)"),
            _   => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Did you parse the channel mode correctly?"))
            },
        };
        // J
        // (5,4) = mode extension (only if channel_mode = joint stereo)
        // let mode_ext = IIJJ & 0x3;

        // bits 3-0 are not pertinent
        
        unsafe { header_count += 1; }
        println!("");
        Ok(Header::format(
            version,
            layer,
            not_protected,
            bitrate,
            sr,
            padded,
            channel_mode,
        ))
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
    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let buf = buf_file(path)?;
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

    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let buf = buf_file(path)?;
        let mut reader = buf.iter().copied();
 
        // read byes little-endian
        let le: bool = true;     

        // RIFF
        print_id(&mut reader, 4);

        let riff_size: u32 = parse_bytes(&mut reader, 4, le)?;
        println!("Chunk size: {riff_size}");

        // WAVE
        print_id(&mut reader, 4);

        println!("");

        // "fmt "
        print_id(&mut reader, 4);        

        let fmt_size: u32 = parse_bytes(&mut reader, 4, le)?;
        println!("Chunk size: {}", fmt_size);

        let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut reader, 2, le)?)
        else {
            return Err(io::Error::new(io::ErrorKind::Unsupported,"Unrecognized format"));
        };
        
        println!("Format code: {fmt_tag:?}");

        let n_chan: u32 = parse_bytes(&mut reader, 2, le)?;
        println!("Num channels: {n_chan}");

        let sample_rate: u32 = parse_bytes(&mut reader, 4, le)?;
        println!("Sample rate: {sample_rate}");

        let data_rate: u32 = parse_bytes(&mut reader, 4, le)?;
        println!("Ave bytes /sec: {data_rate}");

        let data_blk_sz: u32 = parse_bytes(&mut reader, 2, le)?;
        println!("Block size: {data_blk_sz}");

        let bits_per: u32 = parse_bytes(&mut reader, 2, le)?;
        println!("Bits per sample: {bits_per}");

        // if !WaveFormatPcm (i.e. is extensible)
        if fmt_size >= 18 {
            // extension is either 0 or 22
            let cb_size: u32 = parse_bytes(&mut reader, 2, le)?;
            println!("Extension size: {cb_size}");

            if cb_size > 0 {
                let valid_bits: u32 = parse_bytes(&mut reader, 2, le)?;
                println!("Valid bits per sample: {valid_bits}");

                let dw_channel_mask: u32 = parse_bytes(&mut reader, 4, le)?;
                println!("Speaker position mask: {dw_channel_mask}");

                let old_fmt: u32 = parse_bytes(&mut reader, 2, le)?;
                println!("GUID: {old_fmt}");

                // skip over Microsoft stuff
                // TODO: compare against audio media subtype
                for _ in 0..14 {
                    if let Some(b) = reader.next() {}
                }
                print!("\n");
            }
        }
       
        Ok(Vec::<u8>::new())
    }
} // end pub mod wav
