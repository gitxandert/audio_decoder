# BLAST: Bash-Like Audio Scripting Tool

This project aims to facilitate real-time algorithmic audio in Rust. It features a Bash-like scripting language that is interpreted by a REPL.

## Dependencies

One of the goals of this project is to realize as many features as possible with as few dependencies as possible. It currently only uses the `alsa-sys` and `libc` crates for interaction with OS audio and terminal internals.

## Modules

**src/main.rs**:
- executes audio parsing library functions through extension matching
- catches unsupported audio formats

**src/lib.rs**:
- exposes modules to main.rs and hosts testing

**src/audio_processing**  
- pre-parses all audio files in the assets/ folder
- configures ALSA according to a consensus based on the audio files' properties (namely sample rate and number of channels)
- interacts directly with hardware and the DMA buffer for low-latency writes
- processes Commands separately from audio thread for string parsing, hashmap operations, and robust error-handling
- uses terminal in raw mode for custom terminal rendering
- implements fast RNG with xoroshiro128+ generation, Lemire's fast modulo, and architecture-specific seeding

**src/file_parsing**:
- mpeg
  - parses MPEG frames
  - TODO: implement actual decoding of compressed data  
- wav
  - parses RIFF, fmt, and data chunks sequentially
- aiff
  - parses FORM, COMM, and SSND chunks sequentially
- decode_helpers  
  - implements custom DecodeErrors and DecodeResult for in-memory file parsing  
  - provides AudioFile struct to return necessary data for audio APIs

## Documents consulted

**Audio specs**:  
- [DataVoyage's informal MPEG overview](http://mpgedit.org/mpgedit/mpeg_format/mpeghdr.htm#MPEG%20HEADER)  
- [Original AIFF-1.3 specification](https://mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf)  
- [McGill on WAVE](https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/WAVE.html)
