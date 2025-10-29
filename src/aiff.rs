use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::ops::{Shl, BitOr, AddAssign};

use crate::{parse_bytes, parse_ieee_extended, print_id};

pub mod aiff {
    use super::*;

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
