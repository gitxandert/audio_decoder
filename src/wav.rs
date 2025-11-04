use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};
use crate::decode_helpers::{AudioFile, DecodeError, DecodeResult};

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

pub fn print_id(vec: &mut Vec<u8>, start: &mut usize, end: &mut usize) -> DecodeResult<()> {
    *end += 4;

    for i in *start..*end {
        let c = match vec.get(i) {
            Some(val)   => val,
            None    => return Err(DecodeError::UnexpectedEof),
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

    // little-endian
    let mut shift: u32 = 0;
    for i in *start..*end {
        let b: u8 = match bytes.get(i) {
            Some(val) => *val,
            None => return Err(DecodeError::UnexpectedEof),
        };
    
        value += (b as u32) << shift;

        shift += 8;
    }

    *start = *end;

    Ok(value)
}

pub fn parse(path: &str) -> DecodeResult<AudioFile> {
    let mut f = File::open(path)?;
    let mut reader = Vec::new();
    f.read_to_end(&mut reader)?;

    let mut start: usize= 0;
    let mut end: usize = 0;

    // RIFF
    // (print_id always increments end by four before printing
    //  and sets start to end afterward)
    print_id(&mut reader, &mut start, &mut end)?;

    // (parse_bytes increments end by the integer argument
    //  before decoding the reader from start to end
    //  and sets start to end afterward))
    let riff_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
    println!("Chunk size: {riff_size}");

    // WAVE
    print_id(&mut reader, &mut start, &mut end)?;

    println!("");

    // "fmt "
    print_id(&mut reader, &mut start,&mut end)?;        

    let fmt_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
    println!("Chunk size: {}", fmt_size);

    let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut reader, &mut start, &mut end, 2)?.try_into().unwrap())
    else {
        return Err(DecodeError::UnsupportedFormat(String::from("Unrecognized format tag")));
    };
    
    println!("Format code: {fmt_tag:?}");

    let num_channels: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
    println!("Num channels: {num_channels}");

    let sample_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
    println!("Sample rate: {sample_rate}");

    let data_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
    println!("Ave bytes /sec: {data_rate}");

    let data_blk_sz: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
    println!("Block size: {data_blk_sz}");

    let bits_per_sample: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
    println!("Bits per sample: {bits_per_sample}");

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
                end += i;
                print!("{}", reader[end]);
            }
            start = end;
            print!("\n");
        }
    }

    println!("");

    //
    // TODO: parse "fact" chunk if non-PCM (and if exists)
    //

    // "data"
    print_id(&mut reader, &mut start, &mut end)?;
    let data_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
    println!("Data size: {data_size}");
   
    let mut samples: Vec<u8> = vec![0u8; data_size as usize];
    
    end += data_size as usize;
    for i in start..end {
        let s = match reader.get(i) {
            Some(val) => *val,
            None => return Err(DecodeError::UnexpectedEof),
        };
        samples.push(s);
    }

    Ok(AudioFile::new("wav", sample_rate, num_channels, bits_per_sample, samples))
}
