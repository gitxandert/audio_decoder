use alsa::{Direction, ValueOr};
use alsa::pcm::{PCM, HwParams, Format, Access, State};

use crate::decode_helpers::AudioFile;

pub fn play_file(af: AudioFile) {
    /*
     * struct AudioFile {
     *     format: String,      // file type (WAV, AIFF)
     *     sample_rate: u32,
     *     num_channels: u32,
     *     bits_per_sample: u32,
     *     samples: Vec<u8>,
     * }
    */
    // Open default playback device
    let pcm = PCM::new("default", Direction::Playback, false).unwrap();

    // Set hardware parameters
    let hwp = HwParams::any(&pcm).unwrap();
    hwp.set_channels(af.num_channels).unwrap();
    hwp.set_rate(af.sample_rate, ValueOr::Nearest).unwrap();

    hwp.set_format(Format::S16LE).unwrap();

    hwp.set_access(Access::RWInterleaved).unwrap();
    pcm.hw_params(&hwp).unwrap();
    let io = pcm.io_i16().unwrap();

    // Make sure we don't start the stream too early
    let period_size = hwp.get_period_size().unwrap();
    let buffer_size = hwp.get_buffer_size().unwrap();
    let hwp = pcm.hw_params_current().unwrap();
    let swp = pcm.sw_params_current().unwrap();
    swp.set_avail_min(period_size);
    swp.set_start_threshold(period_size).unwrap();
    pcm.sw_params(&swp).unwrap();

    println!("period size = {period_size}, buffer size = {buffer_size}");
    println!("rate = {}, chans = {}, fmt={:?}",
        hwp.get_rate().unwrap(),
        hwp.get_channels().unwrap(),
        hwp.get_format().unwrap());

    for chunk in af.samples.chunks(period_size as usize * af.num_channels as usize) {
        io.writei(chunk).unwrap_or_else(|err| {
            if err.errno() == 32 {
                pcm.prepare().unwrap();
                0
            } else { panic!("ALSA error: {err}"); }
        });
    }

    println!("After write: {:?}", pcm.state());
    
    pcm.drain().unwrap();
}
