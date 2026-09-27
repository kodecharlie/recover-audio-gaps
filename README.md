# recover-audio-gaps
This Rust package takes as input a WAV file containing 16-bit, signed PCM audio data sampled at 24 kHz.  It identifies episodes of white noise, and replaces those interruptions with silences.

# Generate WAV file with white-noise interruptions.
To come up with test data, we rely on Python logic under `generate/inject-noise.py`.

## Install dependencies
You will need to install Python dependencies before running `inject-noise.py`.  For example:
```
pip install edge-tts pydub
```
This is not an exhaustive list of dependencies.  You will need to figure out relevant dependencies for your environment.

Use your best judgment here.  Some use virtual environments for segregating Python executions.

## Execution
Run this command:
```
cd generate
python3 inject-noise.py
```
which should produce the WAV file `professor_1930s.wav`.

## Install and use sox to play back audio
If `sox` is installed on your system, you should be able to play back audio like this:
```
$ play professor_1930s.wav 

professor_1930s.wav:

 File Size: 3.36M     Bit Rate: 384k
  Encoding: Signed PCM    
  Channels: 1 @ 16-bit   
Samplerate: 24000Hz      
Replaygain: off         
  Duration: 00:01:10.00  

In:2.93% 00:00:02.05 [00:01:07.95] Out:90.0k [   -==|==-   ] Hd:3.0 Clip:0 
```

# Run Rust program to stream audio sans white noise interruptions
Run this command:
```
$ cargo run --release -- --in wav/professor_1930s.wav
    Finished `release` profile [optimized] target(s) in 0.17s
     Running `target/release/noise --in wav/professor_1930s.wav`
Streaming 'wav/professor_1930s.wav' seamlessly at 24000 Hz...
...
```
