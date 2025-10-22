# Audio Decoder

This decoder will parse WAV, MP3, and AIFF data, for integration into my [audio scripting project](https://github.com/gitxandert/audio_scripting). 

## Current functionality

src/main.rs:
- executes library functions through extension matching
- catches unsupported formats

src/lib.rs:
- mpeg
    - parses frame headers
- wav
    - parses RIFF and fmt chunks
