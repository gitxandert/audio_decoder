use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};

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

    pub fn print_id(vec: &mut Vec<u8>, start: &mut usize, end: &mut usize) {
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

        // little-endian
        let mut shift: u32 = 0;
        for i in start..end {
            let b: u8 = bytes[i]?;
        
            value += b as u32 << (shift as u32 * 8);

            shift += 1;
        }

        start = end;

        Ok(value)
    }

    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let mut f = File::open(path)?;
        let mut reader = Vec::new();
        f.read_to_end(&mut reader)?;

        let mut start: u32 = 0;
        let mut end: u32 = 0;

        // RIFF
        // (print_id always increments end by four before printing
        //  and sets start to end afterward)
        print_id(&mut reader, &mut start, &mut end);

        // (parse_bytes increments end by the integer argument
        //  before decoding the reader from start to end
        //  and sets start to end afterward))
        let riff_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Chunk size: {riff_size}");

        // WAVE
        print_id(&mut reader, &mut start, &mut end);

        println!("");

        // "fmt "
        print_id(&mut reader, &mut start, &mut end);        

        let fmt_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Chunk size: {}", fmt_size);

        let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut reader, &mut start, &mut end, 2)?)
        else {
            return Err(io::Error::new(io::ErrorKind::Unsupported,"Unrecognized format"));
        };
        
        println!("Format code: {fmt_tag:?}");

        let n_chan: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
        println!("Num channels: {n_chan}");

        let sample_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Sample rate: {sample_rate}");

        let data_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
        println!("Ave bytes /sec: {data_rate}");

        let data_blk_sz: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
        println!("Block size: {data_blk_sz}");

        let bits_per: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
        println!("Bits per sample: {bits_per}");

        // if !WaveFormatPcm (i.e. is extensible)
        if fmt_size >= 18 {
            // extension is either 0 or 22
            let cb_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
            println!("Extension size: {cb_size}");

            if cb_size > 0 {
                let valid_bits: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
                println!("Valid bits per sample: {valid_bits}");

                let dw_channel_mask: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
                println!("Speaker position mask: {dw_channel_mask}");

                let old_fmt: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
                println!("GUID: {old_fmt}");

                // skip over Microsoft stuff
                // TODO: compare against audio media subtype
                for i in 0..14 {
                    print("{}", reader[end + i]);
                }
                print!("\n");
            }
        }
       
        Ok(Vec::<u8>::new())
    }
} // end pub mod wav
