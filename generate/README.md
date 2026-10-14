# generate
These two Python scripts generate WAV files. Google Gemini generated the logic.

## professor.py ... use Google-Text-To-Speech
Install dependencies:
```
pip install audioop-lts
pip install gTTS pydub
```
Run:
```
python3 professor.py
```
to generate the WAV file.

## inject-noise.py ... use Microsoft / Azure Text-To-Speech
Install dependencies:
```
pip install audioop-lts
pip install edge-tts pydub
```
Run:
```
python3 inject-noise.py
```
to generate a WAV file with two bursts of white noise mixed into the original audio samples.
