# Audio Decoder

This decoder will parse WAV, MP3, and AIFF data, for integration into my [audio scripting project](https://github.com/gitxandert/audio_scripting). 

## Current functionality

src/main.rs:
- executes library functions through extension matching
- catches unsupported formats

src/lib.rs:
- mpeg
    - parses frames by:
        1. scanning for any two bytes that look like frame sync
        2. storing possible headers and vectors of indices in a hashmap
        3. sorting possible headers from most to least frequent
        4. extracting most common valid header as a reference header
        5. comparing all other valid headers to the reference
        6. extracting data from file according to the frame lengths of the valid headers
- aiff
    - parses FORM, COMM, and SSND chunks
- wav
    - parses RIFF and fmt chunks

## Documents consulted

[DataVoyage's informal MPEG overview](http://mpgedit.org/mpgedit/mpeg_format/mpeghdr.htm#MPEG%20HEADER)  
[Original AIFF-1.3 specification](https://mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf)  
[McGill on WAVE](https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/WAVE.html)  
