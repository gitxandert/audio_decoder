use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};

use crate::{parse_bytes, parse_ieee_extended, print_id};

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
        let mut f = File::open(path)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;

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
