use std::fs::File;
use std::io::{self, Read, SeekFrom};
use std::collections::HashMap;

pub mod mpeg {
    use super::*;

    // iterate through frames by frame size
    pub fn parse(path: &str) -> io::Result<Vec<u8>> {
        let mut f = File::open(path)?;
        let mut reader = Vec::new();
        f.read_to_end(&mut reader)?;

        let file_len = reader.len();
        let mut cur: usize = 0;
        let mut possibles: HashMap<usize, Vec<usize>> = HashMap::new();

        while cur < file_len {
            if let b = reader[cur] {
                if b == 0xFF {
                    if reader[cur + 1] & 0xE0 == 0xE0 {
                        let fp = cur;
                        let mut supb: usize = 0;
                        supb = ((reader[cur] as usize) << 24);
                        cur += 1;
                        if cur >= file_len {
                            break;
                        }
                        supb |= ((reader[cur] as usize) << 16);
                        cur += 1;
                        if cur >= file_len {
                            break;
                        }
                        supb |= ((reader[cur] as usize) << 8);
                        cur += 1;
                        if cur >= file_len {
                            break;
                        }
                        supb |= reader[cur] as usize;
                        possibles.entry(supb).or_insert(vec![fp]).push(fp);
                        cur += 1;
                    } else {
                        cur += 1;
                    }
                } else {
                    cur += 1;
                }
            } else {
                break;
            }
        }
        
        let mut vecs: Vec<(&usize, &Vec<usize>)> = possibles.iter().collect();
        vecs.sort_by(|a, b| {
            let al = a.1.len();
            let bl = b.1.len();
            bl.cmp(&al)
        });
        for i in 0..5 {
            println!("Value: {:#X}\tInstances: {}", vecs[i].0, vecs[i].1.len());
            match parse_header(vecs[i].0) {
                Ok((v, l, p, br, sr, pd, cm)) => {
                    let header = Header::format(vecs[i].1[0], v, l, p, br, sr, pd, cm);
                },
                Err(error) => {
                    eprintln!("ERROR: {error}");
                    println!("");
                }
            };
        }
        Ok(Vec::<u8>::new())
    }

    #[derive(Debug)]
    struct Header {
       file_pos: usize,
       version: f32,
       layer: i32,
       protected: bool,
       bitrate: u32,
       sr: f64,
       padded: bool,
       channel_mode: u8,
    }

    impl Header {
        fn format(file_pos: usize, version: u8, layer: u8, not_protected: u8, bitrate: u32, sr: f64, padded: u8, channel_mode: u8) -> Self {
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
                file_pos,
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
        let (_, layer, protected, br, sr, _, _) = header.barf();
       
        println!("br = {br} sr = {sr}");
        let br: f64 = br as f64 * 1000f64;
        let frame_len: f64 = match layer {
            3 => 144f64 * br as f64 / sr,
            2 => 144f64 * br as f64 / sr,
            1 => (12f64 * br as f64 / sr) * 4f64,
            _ => {
                return Err(io::Error::new(io::ErrorKind::Unsupported, "Cannot parse reserved layer"))
            }
        };

        let CRC = match protected {
            true => 20,
            false => 4,
        };

        // subtract the header
        Ok(frame_len as u32 - CRC)
    }

    static BITRATES: [[u32; 5]; 15] = [
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
        [0,   0,    0,    0,    0,],
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

    fn match_sr(FFGH: &u8, v_id: &u8) -> f64 {
        let base: f64 = match v_id {
            0x3 => 32000f64,
            0x2 => 16000f64,
            0x0 => 8000f64,
            _   => 0f64,
        };

        let FF = FFGH >> 2;
        let sr: f64 = match FF {
            0x0 => base * 1.378125f64,
            0x1 => base * 1.5f64,
            0x2 => base,
            _   => 0f64,
        };

        sr
    }

    fn skiparound(reader: &mut Vec<u8>, cur: &mut usize) {
        loop {
            let mut input = String::new();
            io::stdin().read_line(&mut input).expect("Failure");
            let input = input.trim();
            let isok = input.parse::<i32>().is_ok();
            if isok {
                let sign = input.chars().nth(0).unwrap();
                if sign == '-' {
                    let parsed = &input[1..].parse::<usize>().unwrap();
                    *cur -= parsed;
                } else {
                    *cur += input.parse::<usize>().unwrap();
                }
                println!("Val at {}: {:#X}", cur, reader[*cur]);
            }
            else {
                if input == "q" {
                    break;
                } else if input == "n" {
                    *cur += 1;
                } else if input == "b" {
                    *cur -= 1;
                } else if input == "f-" {
                    *cur -= 1;
                    let mut count = 1;
                    loop {
                        while reader[*cur] != 0xFF {
                            *cur -= 1;
                            count += 1;
                        }
                        if reader[*cur + 1] & 0xE0 == 0xE0 {
                            break;
                        } else {
                            *cur -= 1;
                            count += 1;
                        }
                    }
                    println!("Skipped backward {count} times");
                } else if input == "f" {
                    *cur += 1;
                    let mut count = 1;
                    loop {
                        while reader[*cur] != 0xFF {
                            *cur += 1;
                            count += 1;
                        }
                        if reader[*cur + 1] & 0xE0 == 0xE0 {
                            break;
                        } else {
                            *cur += 1;
                            count += 1;
                        }
                    }                   
                    println!("Skipped ahead {count} times");
                }
                println!("Val at {}: {:#X}", cur, reader[*cur]);
            }
        }
    }

    // cur is set at the fourth byte in the header
    fn parse_header(bytes: &usize) -> io::Result<(u8, u8, u8, u32, f64, u8, u8)> {
        let unex_eof = io::Error::new(io::ErrorKind::UnexpectedEof, "EOF");
        
        let AAAB_BCCD = (bytes >> 16) as u8 else { return Err(unex_eof) };
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
        
        let EEEE_FFGH = (bytes >> 8) as u8 else { return Err(unex_eof) };
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
            bitrate = match_bitrate(EEEE - 1, &version, &layer);
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
        let IIJJ_KLMM = *bytes as u8 else { return Err(unex_eof) };
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
        
        println!("");
        Ok((
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
