# GART: Generative Audio in Real Time

This project aims to facilitate real-time, generative audio in Rust. It will feature a scripting language that can be both interpreted and compiled into Rust code.

## Dependencies

One of the goals of this project is to realize as many features as possible with as few dependencies as possible. It currently only uses the `alsa` and `libc` crates for interaction with OS audio and terminal internals.

## Modules

**src/main.rs**:
- executes audio parsing library functions through extension matching
- catches unsupported audio formats

**src/lib.rs**:
- exposes modules to main.rs and hosts testing

**src/playback.rs**  
- configures ALSA according to (currently) a single audio file's parameters
- interacts directly with hardware for low-latency buffering
- interprets rudimentary playback commands through a REPL
- uses terminal in raw mode for custom output to the screen

**parsing modules**:
- mpeg
  - parses frames by:
    <ol type="1">
      <li>scanning for any two bytes that look like frame sync</li>
      <li>storing possible headers and vectors of indices in a hashmap</li>
      <li>sorting possible headers from most to least frequent</li>
      <li>extracting most common valid header as a reference header</li>
      <li>comparing all other valid headers to the reference</li>
      <li>extracting data from file according to the frame lengths of the valid headers</li>
    </ol>
  - TODO: implement actual decoding of compressed data  
- wav
  - parses RIFF, fmt, and data chunks sequentially
  - returns sample_rate, num_channels, bits_per_sample, and samples (little-endian) in an AudioFile struct
- aiff
  - parses FORM, COMM, and SSND chunks sequentially
  - returns sample_rate, num_channels, bits_per_sample, and samples (big-endian) in an AudioFile struct
- decode_helpers  
  - implements custom DecodeErrors and DecodeResult for in-memory file parsing  
  - provides AudioFile struct to return necessary data for audio APIs, including:  
    - sample rate  
    - number of channels  
    - bits per sample  
    - extracted samples

## Documents consulted

**Audio specs**:  
- [DataVoyage's informal MPEG overview](http://mpgedit.org/mpgedit/mpeg_format/mpeghdr.htm#MPEG%20HEADER)  
- [Original AIFF-1.3 specification](https://mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf)  
- [McGill on WAVE](https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/WAVE.html)
