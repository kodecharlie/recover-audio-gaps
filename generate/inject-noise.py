import array
import asyncio
import io
import random
import struct
import wave
import edge_tts
from pydub import AudioSegment

# 1. Monologue Text
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

# High-fidelity male neural voice
VOICE = "en-US-ChristopherNeural"

# Audio specifications: 24 kHz preserves high-frequency formants and vocal clarity
SAMPLE_RATE = 24000  # 24 kHz
TARGET_DURATION_SEC = 90.0  # Total WAV container length
TOTAL_SAMPLES = int(SAMPLE_RATE * TARGET_DURATION_SEC)  # 2,160,000 samples

# Noise burst settings
NOISE_BURSTS = [
    {"time_sec": 10.0, "duration_ms": 80},
    {"time_sec": 60.0, "duration_ms": 80},
]


async def generate_speech_mp3(text: str, voice_name: str) -> io.BytesIO:
    """Generates speech via edge-tts with a slightly deliberate rate for academic delivery."""
    # rate="-5%" slows down speech slightly without changing voice pitch
    communicate = edge_tts.Communicate(text, voice_name, rate="-5%")
    audio_bytes = bytearray()
    async for chunk in communicate.stream():
        if chunk["type"] == "audio":
            audio_bytes.extend(chunk["data"])
    return io.BytesIO(audio_bytes)


print(f"Generating speech audio via Edge TTS ({VOICE})...")
mp3_fp = asyncio.run(generate_speech_mp3(MONOLOGUE, VOICE))

# Convert MP3 to 24kHz Mono Signed 16-bit PCM using pydub
audio = AudioSegment.from_file(mp3_fp, format="mp3")
audio = audio.set_frame_rate(SAMPLE_RATE).set_channels(1)

# Fit/pad audio to 90 seconds
target_length_ms = int(TARGET_DURATION_SEC * 1000)
if len(audio) < target_length_ms:
    audio = audio + AudioSegment.silent(duration=target_length_ms - len(audio))
else:
    audio = audio[:target_length_ms]

# Extract raw signed 16-bit PCM samples into an array ('h' = signed 16-bit short)
samples = array.array("h", audio.raw_data)

# Pad or trim sample array to match exact 90-second total sample count
if len(samples) < TOTAL_SAMPLES:
    samples.extend([0] * (TOTAL_SAMPLES - len(samples)))
else:
    samples = samples[:TOTAL_SAMPLES]

# Inject white noise bursts directly into 16-bit sample range (-32768 to 32767)
print("Injecting white noise interruptions at 10s and 60s...")
for burst in NOISE_BURSTS:
    start_sample = int(burst["time_sec"] * SAMPLE_RATE)
    num_noise_samples = int((burst["duration_ms"] / 1000.0) * SAMPLE_RATE)
    end_sample = min(start_sample + num_noise_samples, TOTAL_SAMPLES)

    for i in range(start_sample, end_sample):
        samples[i] = random.randint(-32768, 32767)

# Export as 16-bit Signed PCM WAV
output_filename = "professor_1930s.wav"
with wave.open(output_filename, "wb") as wav_file:
    wav_file.setnchannels(1)  # Mono
    wav_file.setsampwidth(2)  # 16-bit (2 bytes per sample)
    wav_file.setframerate(SAMPLE_RATE)  # 24000 Hz
    wav_file.writeframes(samples.tobytes())

print(
    f"Successfully generated '{output_filename}' ({TARGET_DURATION_SEC}s, {SAMPLE_RATE}Hz Mono, Signed 16-bit PCM)."
)
