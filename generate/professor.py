import io
import re
from gtts import gTTS
from pydub import AudioSegment

MONOLOGUE = (
    "Good afternoon, class. Today we turn our attention to the American 1930s—a "
    "decade defined by unprecedented economic turmoil, social upheaval, and profound "
    "institutional reform. The stock market crash of late 1929 ushered in the Great "
    "Depression, driving unemployment to nearly twenty-five percent at its peak. "
    "Banks collapsed across the nation, industrial output plummeted, and prolonged "
    "drought devastated the Great Plains, giving rise to the Dust Bowl. Yet, this "
    "era was not merely a chapter of hardship; it fundamentally reshaped the social "
    "contract between the American public and the federal government. Under President "
    "Franklin Delano Roosevelt, the New Deal introduced expansive public works "
    "initiatives, financial regulations, and social safety nets like Social Security. "
    "Concurrently, popular culture offered both escape and solidarity through radio "
    "broadcasts, jazz, and cinema. As we analyze these primary sources, consider how "
    "ordinary citizens navigated systemic collapse to redefine American resilience."
)

TARGET_DURATION_SEC = 90.0
TARGET_DURATION_MS = int(TARGET_DURATION_SEC * 1000)

# 1. Split text into individual sentences to inject natural academic pauses
sentences = [
    s.strip() for s in re.split(r"(?<=[.!?])\s+", MONOLOGUE) if s.strip()
]

# 2. Synthesize each sentence via TTS
sentence_segments = []
for sentence in sentences:
    tts = gTTS(text=sentence, lang="en", slow=False)
    fp = io.BytesIO()
    tts.write_to_fp(fp)
    fp.seek(0)
    seg = AudioSegment.from_file(fp, format="mp3")
    sentence_segments.append(seg)

# 3. Calculate necessary pause duration between sentences to hit exactly 90 seconds
raw_speech_ms = sum(len(seg) for seg in sentence_segments)
num_pauses = len(sentence_segments)  # Includes tail silence
remaining_ms = max(0, TARGET_DURATION_MS - raw_speech_ms)
pause_per_sentence_ms = remaining_ms // num_pauses

# 4. Assemble the audio with calculated silence
final_audio = AudioSegment.silent(duration=0)
for seg in sentence_segments:
    final_audio += seg
    final_audio += AudioSegment.silent(duration=pause_per_sentence_ms)

# Trim or pad to hit 90 seconds exactly
final_audio = final_audio[:TARGET_DURATION_MS]

# 5. Format as clean CD-quality WAV (44.1 kHz, 16-bit Signed PCM, Mono)
final_audio = (
    final_audio.set_frame_rate(44100)
    .set_channels(1)
    .set_sample_width(2)  # 2 bytes = standard 16-bit signed PCM
)

output_filename = "professor_1930s_clean.wav"
final_audio.export(output_filename, format="wav")

print(
    f"Successfully exported '{output_filename}' ({len(final_audio)/1000:.1f}s, 44.1kHz Mono, Signed 16-bit PCM)."
)
