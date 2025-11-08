# Audio Decoder

This decoder will parse WAV, MP3, and AIFF data, for integration into my [audio scripting project](https://github.com/gitxandert/audio_scripting). 

## Current functionality

**src/main.rs**:
- executes library functions through extension matching
- catches unsupported formats

**lib modules**:
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
  - parses RIFF, fmt, and data chunks
  - returns sample_rate, num_channels, bits_per_sample, and samples (little-endian) in an AudioFile struct
- aiff
  - parses FORM, COMM, and SSND chunks
  - returns sample_rate, num_channels, bits_per_sample, and samples (big-endian) in an AudioFile struct
- playback
  - utilizes ALSA crate for simple playback of parsed audio files
  - formats hardware parameters according to AudioFile fields
  - TODO: asynchronous ring buffer for real-time play
- decode_helpers  
  - implements custom DecodeErrors and DecodeResult for in-memory file parsing  
  - provides AudioFile struct to return necessary data for audio APIs, including:  
    - sample rate  
    - number of channels  
    - bits per sample  
    - extracted samples
- lib.rs  
  - exposes modules to main.rs and hosts testing

## Documents consulted

[DataVoyage's informal MPEG overview](http://mpgedit.org/mpgedit/mpeg_format/mpeghdr.htm#MPEG%20HEADER)  
[Original AIFF-1.3 specification](https://mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf)  
[McGill on WAVE](https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/WAVE.html)  
