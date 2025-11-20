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
    }

    *start = *end;


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

    // WAVE
    print_id(&mut reader, &mut start, &mut end)?;

    // "fmt "
    print_id(&mut reader, &mut start,&mut end)?;        

    let fmt_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;

    let Some(fmt_tag) = FormatCode::from_u16(parse_bytes(&mut reader, &mut start, &mut end, 2)?.try_into().unwrap())
    else {
        return Err(DecodeError::UnsupportedFormat(String::from("Unrecognized format tag")));
    };
    
    let num_channels: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;
    
    let sample_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;

    let data_rate: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;

    let data_blk_sz: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;

    let bits_per_sample: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;

    // if !WaveFormatPcm (i.e. is extensible)
    if fmt_size >= 18 {
        // extension is either 0 or 22
        let cb_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;

        if cb_size > 0 {
            let valid_bits: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;

            let dw_channel_mask: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;

            let old_fmt: u32 = parse_bytes(&mut reader, &mut start, &mut end, 2)?;

            // skip over Microsoft stuff
            // TODO: compare against audio media subtype
            for i in 0..14 {
                end += i;
            }
            start = end;
        }
    }


    //
    // TODO: parse "fact" chunk if non-PCM (and if exists)
    //

    // "data"
    print_id(&mut reader, &mut start, &mut end)?;
    let data_size: u32 = parse_bytes(&mut reader, &mut start, &mut end, 4)?;
   
    let mut samples: Vec<i16> = Vec::new();
    
    end += data_size as usize;
    for i in (start..end).step_by(2) {
        let s1 = match reader.get(i) {
            Some(val) => *val,
            None => return Err(DecodeError::UnexpectedEof),
        };
        let s2 = match reader.get(i + 1) {
            Some(val) => *val,
            None => return Err(DecodeError::UnexpectedEof),
        };
 
        samples.push(i16::from_le_bytes([s1, s2]));
    }

    let file_name: &str = match path.rsplit_once(|b: char| b == '.') {
        Some((before, after)) if !before.is_empty() && !after.is_empty() => {
            match before.rsplit_once(|b: char| b == '/') {
                Some((assets, name)) => name,
                None => return Err(DecodeError::InvalidData("File is not nested".to_string())),
            }
        }
        _ => return Err(DecodeError::InvalidData("File has no name".to_string())),
    };

    Ok(AudioFile::new(file_name, "wav", sample_rate, num_channels, bits_per_sample, samples))
}
