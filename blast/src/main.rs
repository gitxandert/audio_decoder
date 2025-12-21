use std::fs;
use std::collections::{HashMap, hash_map::Entry};
use blast::{
    file_parsing::{
        mpeg, aiff, wav,
        decode_helpers::{
            DecodeError, DecodeResult, AudioFile
        },
    },
    audio_processing::runtime::run_blast,
};

fn main() -> DecodeResult<()> {
    let mut tracks = HashMap::<String, AudioFile>::new();
    let mut sample_rates = HashMap::<u32, u32>::new();
    let mut channel_nums = Vec::<u32>::new();

    for entry in fs::read_dir("blast/assets/")? {
        let dir = match entry {
            Ok(pathbuf) => pathbuf,
            Err(error) => {
                println!("Error: {error}");
                continue;
            }
        };

        let pathbuf = dir.path();

        let path = match pathbuf.to_str() {
            Some(valid) => valid,
            None => {
                println!("Error: invalid unicode in '{:?}'", dir.path());
                continue;
            }
        };


        let ext: &str = match path.rsplit_once(|b: char| b == '.') {
            Some((before, after)) if !before.is_empty() && !after.is_empty() => after,
            _ => "",
        };

        let track: AudioFile = match ext {
            /* TODO: figure out actual mpeg decoding...
            "mp3" => {
                match mpeg::parse(path) {
                    Ok(file) => file,
                    Err(error) => {
                        println!("{:?}", error);
                        continue;
                    }
                }
            }
            */
            "wav" => {
                match wav::parse(path) {
                    Ok(file) => file,
                    Err(error) => {
                        println!("{:?}", error);
                        continue;
                    }
                }
            }
            "aif" => {
                match aiff::parse(path) {
                    Ok(file) => file,
                    Err(error) => {
                        println!("{:?}", error);
                        continue;
                    }
                }
            }
            _ => {
                println!("Error: unsupported format for '{}'", path);
                continue;
            }
        };

        *sample_rates.entry(track.sample_rate).or_insert(0) += 1;
        channel_nums.push(track.num_channels);
        
        match tracks.entry(track.file_name.clone()) {
            Entry::Vacant(e) => { e.insert(track);}
            Entry::Occupied(_) => {
                println!("Error: multiple files with the same name {}", track.file_name);
                continue;
            }
        }
    }

    let mutual_rate: u32 = {
        let mut rates: Vec<(&u32, &u32)> = sample_rates.iter().collect();
        rates.sort_by(|(_, v1), (_, v2)| v2.cmp(v1));
        let (key, val) = match rates.get(0) {
            Some((k, v)) => (**k, **v),
            None => {
                println!("Error: problem with deciding sample rate");
                (44100, 0)
            }
        };
       
        println!("Mutual sample_rate: {key}");

        key
    };

    let num_channels: u32 = {
        channel_nums.sort_by(|v1, v2| v2.cmp(v1));
        let val = match channel_nums.get(0) {
            Some(v) => *v,
            None => {
                println!("Error: problem with deciding num channels");
                2
            }
        };

        println!("Num channels: {val}");

        val
    };

    println!("Loaded tracks [");
    for (track, _) in &tracks {
        println!("\t{}", track);
    }
    println!("]");

    run_blast(tracks, mutual_rate, num_channels);

    Ok(())
}
