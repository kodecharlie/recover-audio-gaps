use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::WavReader;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::process;
use std::sync::{Arc, Mutex};

// DSP & Inpainting Constants
const WINDOW_SIZE: usize = 256;      // ~10.6 ms frame size at 24 kHz
const LPC_ORDER: usize = 16;         // 16th-order LPC for speech formants
const PRE_GAP_CONTEXT: usize = 512;  // ~21.3 ms pre-history for forward LPC/pitch
const POST_GAP_CONTEXT: usize = 512; // ~21.3 ms post-history for backward LPC/pitch

// Noise Detection Thresholds
const ZCR_THRESHOLD: f32 = 0.30;
const RMS_THRESHOLD: f32 = 0.20;
const PEAK_THRESHOLD: f32 = 0.50;

/// Fits LPC predictor coefficients (a_1..a_order) using Levinson-Durbin recursion
fn levinson_durbin(autocorr: &[f32], order: usize) -> Vec<f32> {
    let mut a = vec![0.0; order + 1];
    let mut a_prev = vec![0.0; order + 1];
    let mut e = autocorr[0];

    if e.abs() < 1e-7 {
        return vec![0.0; order];
    }

    for i in 1..=order {
        let mut lambda = autocorr[i];
        for j in 1..i {
            lambda -= a_prev[j] * autocorr[i - j];
        }
        let k = lambda / e;
        a[i] = k;
        for j in 1..i {
            a[j] = a_prev[j] - k * a_prev[i - j];
        }
        e *= 1.0 - k * k;
        a_prev = a.clone();
    }

    a[1..=order].to_vec()
}

/// Estimates pitch period P (in samples) using normalized autocorrelation (50 Hz to 400 Hz)
fn estimate_pitch_period(samples: &[f32], sample_rate: u32) -> usize {
    let min_lag = (sample_rate / 400) as usize; // 400 Hz max
    let max_lag = (sample_rate / 50) as usize;  // 50 Hz min

    if samples.len() < max_lag * 2 {
        return 120; // Default fallback (~200 Hz at 24 kHz)
    }

    let mut best_lag = min_lag;
    let mut max_corr = -1.0f32;
    let n = samples.len() / 2;

    for lag in min_lag..=max_lag.min(n) {
        let mut corr = 0.0;
        let mut energy1 = 0.0;
        let mut energy2 = 0.0;

        for i in 0..n {
            let s1 = samples[samples.len() - n + i];
            let s2 = samples[samples.len() - n + i - lag];
            corr += s1 * s2;
            energy1 += s1 * s1;
            energy2 += s2 * s2;
        }

        let norm_denom = (energy1 * energy2).sqrt();
        if norm_denom > 1e-6 {
            let norm_corr = corr / norm_denom;
            if norm_corr > max_corr {
                max_corr = norm_corr;
                best_lag = lag;
            }
        }
    }

    best_lag
}

/// Inpaints missing gap samples using Forward-Backward LPC and Pitch Replication
fn inpaint_gap(pre_history: &[f32], post_future: &[f32], gap_len: usize, sample_rate: u32) -> Vec<f32> {
    if pre_history.is_empty() || post_future.is_empty() || gap_len == 0 {
        return vec![0.0; gap_len];
    }

    // 1. Estimate Pitch Period
    let pitch_period = estimate_pitch_period(pre_history, sample_rate);

    // 2. Calculate Forward LPC Coefficients (from pre-gap speech)
    let mut autocorr_fwd = vec![0.0; LPC_ORDER + 1];
    for lag in 0..=LPC_ORDER {
        for i in lag..pre_history.len() {
            autocorr_fwd[lag] += pre_history[i] * pre_history[i - lag];
        }
    }
    let lpc_fwd = levinson_durbin(&autocorr_fwd, LPC_ORDER);

    // 3. Forward Extrapolation Buffer: [pre_history | gap]
    let pre_len = pre_history.len();
    let mut fwd_buf = vec![0.0; pre_len + gap_len];
    fwd_buf[..pre_len].copy_from_slice(pre_history);

    for n in 0..gap_len {
        let idx = pre_len + n;
        let mut lpc_val = 0.0;
        for j in 0..LPC_ORDER {
            lpc_val += lpc_fwd[j] * fwd_buf[idx - 1 - j];
        }

        let pitch_val = if idx >= pitch_period {
            fwd_buf[idx - pitch_period]
        } else {
            0.0
        };

        // Hybrid excitation: 60% spectral envelope + 40% pitch harmonic continuation
        fwd_buf[idx] = 0.6 * lpc_val + 0.4 * pitch_val;
    }

    // 4. Calculate Backward LPC Coefficients (from post-gap speech)
    let mut autocorr_bwd = vec![0.0; LPC_ORDER + 1];
    for lag in 0..=LPC_ORDER {
        for i in 0..(post_future.len() - lag) {
            autocorr_bwd[lag] += post_future[i] * post_future[i + lag];
        }
    }
    let lpc_bwd = levinson_durbin(&autocorr_bwd, LPC_ORDER);

    // 5. Backward Extrapolation Buffer: [gap | post_future]
    let post_len = post_future.len();
    let mut bwd_buf = vec![0.0; gap_len + post_len];
    bwd_buf[gap_len..].copy_from_slice(post_future);

    for n in (0..gap_len).rev() {
        let mut lpc_val = 0.0;
        for j in 0..LPC_ORDER {
            lpc_val += lpc_bwd[j] * bwd_buf[n + 1 + j];
        }

        let pitch_val = if n + pitch_period < bwd_buf.len() {
            bwd_buf[n + pitch_period]
        } else {
            0.0
        };

        bwd_buf[n] = 0.6 * lpc_val + 0.4 * pitch_val;
    }

    // 6. Crossfade Forward and Backward Synthesizations Across the Gap
    let mut output = vec![0.0; gap_len];
    for n in 0..gap_len {
        let w = (gap_len as f32 - 1.0 - n as f32) / (gap_len as f32 - 1.0).max(1.0);
        let inpainted_sample = w * fwd_buf[pre_len + n] + (1.0 - w) * bwd_buf[n];
        output[n] = inpainted_sample;
    }

    // 7. Apply Boundary Crossfading to avoid transient clicks at gap edges
    const CROSSFADE_LEN: usize = 48; // ~2ms fade
    let fade_len = CROSSFADE_LEN.min(gap_len / 2);

    for i in 0..fade_len {
        let fade_in = i as f32 / fade_len as f32;
        let fade_out = 1.0 - fade_in;

        // Smooth start boundary with pre-gap last sample
        output[i] = fade_in * output[i] + fade_out * pre_history[pre_len - 1];
        // Smooth end boundary with post-gap first sample
        let end_idx = gap_len - 1 - i;
        output[end_idx] = fade_in * output[end_idx] + fade_out * post_future[0];
    }

    output
}

struct StreamDeNoiserInpainter {
    reader: WavReader<BufReader<File>>,
    sample_rate: u32,
    total_samples: u32,
    raw_stream: Vec<f32>,
    read_pos: usize,
}

impl StreamDeNoiserInpainter {
    pub fn new(file_path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut reader = WavReader::open(file_path)?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let total_samples = reader.len();

        let mut raw_stream = Vec::with_capacity(total_samples as usize);
        for sample in reader.samples::<i16>() {
            if let Ok(s) = sample {
                raw_stream.push(s as f32 / 32768.0);
            }
        }

        let mut denoiser = Self {
            reader,
            sample_rate,
            total_samples,
            raw_stream,
            read_pos: 0,
        };

        denoiser.process_and_inpaint_entire_stream();
        Ok(denoiser)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn duration_secs(&self) -> u64 {
        (self.total_samples as u64) / (self.sample_rate as u64)
    }

    fn is_scratchy_noise(&self, window: &[f32]) -> bool {
        if window.is_empty() {
            return false;
        }

        let mut zero_crossings = 0;
        for i in 1..window.len() {
            if (window[i] >= 0.0 && window[i - 1] < 0.0)
                || (window[i] < 0.0 && window[i - 1] >= 0.0)
            {
                zero_crossings += 1;
            }
        }
        let zcr = zero_crossings as f32 / window.len() as f32;

        let sum_squares: f32 = window.iter().map(|s| s * s).sum();
        let rms = (sum_squares / window.len() as f32).sqrt();
        let max_peak = window.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

        // zcr > ZCR_THRESHOLD && rms > RMS_THRESHOLD && max_peak > PEAK_THRESHOLD
        zcr > ZCR_THRESHOLD && rms > RMS_THRESHOLD
    }

    /// Scans audio stream, isolates noise bursts, and inpaints them with synthesized speech
    fn process_and_inpaint_entire_stream(&mut self) {
        let stream_len = self.raw_stream.len();
        let mut i = 0;

        while i + WINDOW_SIZE <= stream_len {
            let window = &self.raw_stream[i..i + WINDOW_SIZE];

            if self.is_scratchy_noise(window) {
                let gap_start = i;
                let mut gap_end = i + WINDOW_SIZE;

                // Extend gap end until clean speech resumes
                while gap_end + WINDOW_SIZE <= stream_len {
                    let next_win = &self.raw_stream[gap_end..gap_end + WINDOW_SIZE];
                    if self.is_scratchy_noise(next_win) {
                        gap_end += WINDOW_SIZE;
                    } else {
                        break;
                    }
                }

                // Add 128 samples (~5ms) holdover margin to capture noise trailing edges
                gap_end = (gap_end + 128).min(stream_len);
                let gap_len = gap_end - gap_start;

                // Extract Context Buffers
                let pre_start = gap_start.saturating_sub(PRE_GAP_CONTEXT);
                let pre_history = self.raw_stream[pre_start..gap_start].to_vec();

                let post_end = (gap_end + POST_GAP_CONTEXT).min(stream_len);
                let post_future = self.raw_stream[gap_end..post_end].to_vec();

                println!(
                    "Inpainting noise burst at {:.2}s (duration: {:.1}ms)...",
                    gap_start as f32 / self.sample_rate as f32,
                    (gap_len as f32 / self.sample_rate as f32) * 1000.0
                );

                // Synthesize signal across gap
                let inpainted_samples = inpaint_gap(&pre_history, &post_future, gap_len, self.sample_rate);

                // Replace noise burst in audio stream
                self.raw_stream[gap_start..gap_end].copy_from_slice(&inpainted_samples);

                i = gap_end;
            } else {
                i += WINDOW_SIZE;
            }
        }
    }

    pub fn read_next_samples(&mut self, output: &mut [f32]) {
        for sample in output.iter_mut() {
            if self.read_pos < self.raw_stream.len() {
                *sample = self.raw_stream[self.read_pos];
                self.read_pos += 1;
            } else {
                *sample = 0.0;
            }
        }
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

    let denoiser = StreamDeNoiserInpainter::new(&input_path)?;
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

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut guard = denoiser_clone.lock().unwrap();
            guard.read_next_samples(data);
        },
        move |err| eprintln!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;
    println!("Streaming '{}' with LPC + Pitch-Period inpainting at {} Hz...", input_path, sample_rate);

    std::thread::sleep(std::time::Duration::from_secs(duration_secs + 1));

    Ok(())
}