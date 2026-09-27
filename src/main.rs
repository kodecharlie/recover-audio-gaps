use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::WavReader;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::process;
use std::sync::{Arc, Mutex};

/// Buffer and DSP configuration
const WINDOW_SIZE: usize = 256;      // ~10.6ms latency chunk at 24kHz
const ZCR_THRESHOLD: f32 = 0.30;     // White noise zero-crossing threshold
const RMS_THRESHOLD: f32 = 0.20;     // Minimum noise RMS energy
const PEAK_THRESHOLD: f32 = 0.50;    // Peak absolute sample value threshold

struct StreamDeNoiser {
    reader: WavReader<BufReader<File>>,
    sample_rate: u32,
    total_samples: u32,
    lookahead_window: Vec<f32>,
    current_gain: f32,
    noise_holdover_samples: usize,
}

impl StreamDeNoiser {
    pub fn new(file_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let reader = WavReader::open(file_path)?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let total_samples = reader.len();

        let mut denoiser = Self {
            reader,
            sample_rate,
            total_samples,
            lookahead_window: Vec::new(),
            current_gain: 1.0,
            noise_holdover_samples: 0,
        };

        // Prime the lookahead buffer with the first window
        denoiser.lookahead_window = denoiser.read_raw_window();

        Ok(denoiser)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn duration_secs(&self) -> u64 {
        (self.total_samples as u64) / (self.sample_rate as u64)
    }

    /// Fetches the next raw 256-sample chunk from the WAV file
    fn read_raw_window(&mut self) -> Vec<f32> {
        let mut window = Vec::with_capacity(WINDOW_SIZE);
        let mut iter = self.reader.samples::<i16>();

        for _ in 0..WINDOW_SIZE {
            if let Some(Ok(s)) = iter.next() {
                window.push(s as f32 / 32768.0);
            } else {
                window.push(0.0);
            }
        }
        window
    }

    /// Evaluates multi-metric white-noise signatures
    fn is_scratchy_noise(&self, window: &[f32]) -> bool {
        if window.is_empty() {
            return false;
        }

        // 1. Zero-Crossing Rate (ZCR)
        let mut zero_crossings = 0;
        for i in 1..window.len() {
            if (window[i] >= 0.0 && window[i - 1] < 0.0)
                || (window[i] < 0.0 && window[i - 1] >= 0.0)
            {
                zero_crossings += 1;
            }
        }
        let zcr = zero_crossings as f32 / window.len() as f32;

        // 2. RMS Energy
        let sum_squares: f32 = window.iter().map(|s| s * s).sum();
        let rms = (sum_squares / window.len() as f32).sqrt();

        // 3. Peak Absolute Amplitude
        let max_peak = window.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

        zcr > ZCR_THRESHOLD && rms > RMS_THRESHOLD && max_peak > PEAK_THRESHOLD
    }

    /// Processes and de-noises audio using 1-window lookahead and smooth crossfading
    pub fn process_next_window(&mut self) -> Vec<f32> {
        // Fetch new lookahead window and retrieve current playout window
        let new_lookahead = self.read_raw_window();
        let current_raw_window = std::mem::replace(&mut self.lookahead_window, new_lookahead);

        // Analyze both current and upcoming lookahead windows for noise
        let current_is_noise = self.is_scratchy_noise(&current_raw_window);
        let lookahead_is_noise = self.is_scratchy_noise(&self.lookahead_window);

        // If noise is present or approaching, trigger/extend the mute holdover
        if current_is_noise || lookahead_is_noise {
            // Hold over for current window + lookahead window + margin (~20ms extra)
            self.noise_holdover_samples = WINDOW_SIZE * 2 + 128;
        }

        // Crossfade speed: 48 samples = 2.0 ms ramp time at 24 kHz
        const FADE_SAMPLES: f32 = 48.0;
        const GAIN_STEP: f32 = 1.0 / FADE_SAMPLES;

        let mut output_window = Vec::with_capacity(WINDOW_SIZE);

        for &sample in current_raw_window.iter() {
            let target_gain = if self.noise_holdover_samples > 0 {
                0.0
            } else {
                1.0
            };

            // Smoothly slew current gain towards target gain to prevent step clicks
            if self.current_gain < target_gain {
                self.current_gain = (self.current_gain + GAIN_STEP).min(1.0);
            } else if self.current_gain > target_gain {
                self.current_gain = (self.current_gain - GAIN_STEP).max(0.0);
            }

            if self.noise_holdover_samples > 0 {
                self.noise_holdover_samples -= 1;
            }

            output_window.push(sample * self.current_gain);
        }

        output_window
    }
}

fn parse_args() -> Result<String, String> {
    let args: Vec<String> = env::args().collect();
    let mut i = 1;

    while i < args.len() {
        if args[i] == "--in" && i + 1 < args.len() {
            return Ok(args[i + 1].clone());
        } else if args[i].starts_with("--in=") {
            return Ok(args[i]["--in=".len()..].to_string());
        }
        i += 1;
    }

    Err("Usage: cargo run -- --in <path_to_wav>".to_string())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = match parse_args() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("{}", err);
            process::exit(1);
        }
    };

    let denoiser = StreamDeNoiser::new(&input_path)?;
    let sample_rate = denoiser.sample_rate();
    let duration_secs = denoiser.duration_secs();

    let shared_denoiser = Arc::new(Mutex::new(denoiser));

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No audio output device available");

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let denoiser_clone = Arc::clone(&shared_denoiser);
    let mut current_window = Vec::<f32>::new();
    let mut window_idx = 0;

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut guard = denoiser_clone.lock().unwrap();

            for sample_out in data.iter_mut() {
                if window_idx >= current_window.len() {
                    current_window = guard.process_next_window();
                    window_idx = 0;
                }

                *sample_out = current_window[window_idx];
                window_idx += 1;
            }
        },
        move |err| eprintln!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;
    println!("Streaming '{}' seamlessly at {} Hz...", input_path, sample_rate);

    std::thread::sleep(std::time::Duration::from_secs(duration_secs + 1));

    Ok(())
}
