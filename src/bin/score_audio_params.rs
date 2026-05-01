use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use gba_emu::NativeEmulator;

struct Dataset {
    frames: u32,
    path: PathBuf,
    reference: WavData,
}

struct Candidate {
    label: String,
    spec: Option<String>,
}

struct WavData {
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

struct AudioCompareMetrics {
    rmse: f64,
    correlation: f64,
    scale: f64,
    first_nonzero_pair: usize,
    reference_first_nonzero_pair: usize,
    peak: i32,
    reference_peak: i32,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let rom_path = args.next().ok_or_else(|| {
        "usage: score_audio_params <rom> --dataset <frames> <oracle.wav> [--dataset ...] [--candidate \"key=value ...\"] [--candidate-file file]"
            .to_string()
    })?;

    let mut dataset_args = Vec::new();
    let mut candidate_specs = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dataset" => {
                let frames = args
                    .next()
                    .ok_or_else(|| "missing dataset frame count".to_string())?
                    .parse::<u32>()
                    .map_err(|_| "dataset frame count must be an integer".to_string())?;
                let path = PathBuf::from(args.next().ok_or_else(|| "missing dataset wav path".to_string())?);
                dataset_args.push((frames, path));
            }
            "--candidate" => {
                candidate_specs.push(args.next().ok_or_else(|| "missing candidate spec".to_string())?);
            }
            "--candidate-file" => {
                let path = PathBuf::from(args.next().ok_or_else(|| "missing candidate file path".to_string())?);
                candidate_specs.extend(load_candidate_specs(&path)?);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    if dataset_args.is_empty() {
        return Err("at least one --dataset <frames> <oracle.wav> is required".to_string());
    }

    let rom = fs::read(&rom_path).map_err(|e| format!("failed to read ROM: {e}"))?;
    let datasets = dataset_args
        .into_iter()
        .map(|(frames, path)| {
            let reference = read_wav_i16(&path)?;
            Ok(Dataset { frames, path, reference })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut candidates = Vec::new();
    candidates.push(Candidate {
        label: "baseline".to_string(),
        spec: None,
    });
    for spec in candidate_specs {
        candidates.push(Candidate {
            label: compact_candidate_label(&spec),
            spec: Some(spec),
        });
    }

    for (idx, candidate) in candidates.iter().enumerate() {
        let mut aggregate_rmse = 0.0;
        let mut aggregate_peak_overage = 0i32;
        println!("candidate[{idx}] {}", candidate.label);
        for dataset in &datasets {
            let mut emu =
                NativeEmulator::new_with_rom(&rom).ok_or_else(|| "failed to initialize emulator".to_string())?;
            if let Some(spec) = &candidate.spec {
                emu.set_audio_params_for_debug(spec)?;
            }
            for _ in 0..dataset.frames {
                emu.run_frame();
            }
            let audio = emu.take_audio();
            let metrics = compare_audio_metrics(&audio, emu.audio_rate() as u32, &dataset.reference);
            aggregate_rmse += metrics.rmse;
            aggregate_peak_overage += (metrics.peak - metrics.reference_peak).max(0);
            println!(
                "  frames={} rmse={:.6} corr={:.9} scale={:.9} first_nonzero={}/{} peak={}/{} {}",
                dataset.frames,
                metrics.rmse,
                metrics.correlation,
                metrics.scale,
                metrics.first_nonzero_pair,
                metrics.reference_first_nonzero_pair,
                metrics.peak,
                metrics.reference_peak,
                dataset.path.display()
            );
        }
        println!(
            "  aggregate_rmse={:.6} aggregate_peak_overage={}",
            aggregate_rmse, aggregate_peak_overage
        );
    }

    Ok(())
}

fn load_candidate_specs(path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("failed to read candidate file {}: {e}", path.display()))?;
    let mut specs = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !trimmed.contains("dead=") && !trimmed.contains("pregain=") {
            continue;
        }
        specs.push(trimmed.to_string());
    }
    Ok(specs)
}

fn compact_candidate_label(spec: &str) -> String {
    let mut parts = Vec::new();
    for token in spec.split(|ch: char| ch == ',' || ch.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() || !token.contains('=') {
            continue;
        }
        let Some((key, _)) = token.split_once('=') else {
            continue;
        };
        if matches!(key, "total_rmse" | "objective_total" | "total_peak_overage") {
            continue;
        }
        parts.push(token.to_string());
    }
    parts.join(" ")
}

fn read_wav_i16(path: &Path) -> Result<WavData, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read wav {}: {e}", path.display()))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(format!("wav {} must be RIFF/WAVE", path.display()));
    }

    let mut offset = 12usize;
    let mut channels = None;
    let mut sample_rate = None;
    let mut bits_per_sample = None;
    let mut audio_format = None;
    let mut data = None;

    while offset + 8 <= bytes.len() {
        let chunk_id = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        offset += 8;
        if offset + chunk_size > bytes.len() {
            return Err(format!("wav chunk in {} extends past end of file", path.display()));
        }
        let chunk = &bytes[offset..offset + chunk_size];
        match chunk_id {
            b"fmt " => {
                if chunk.len() < 16 {
                    return Err(format!("wav fmt chunk in {} is too small", path.display()));
                }
                audio_format = Some(u16::from_le_bytes(chunk[0..2].try_into().unwrap()));
                channels = Some(u16::from_le_bytes(chunk[2..4].try_into().unwrap()));
                sample_rate = Some(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
                bits_per_sample = Some(u16::from_le_bytes(chunk[14..16].try_into().unwrap()));
            }
            b"data" => data = Some(chunk.to_vec()),
            _ => {}
        }
        offset += chunk_size + (chunk_size & 1);
    }

    if audio_format != Some(1) {
        return Err(format!("wav {} must be PCM", path.display()));
    }
    let channels = channels.ok_or_else(|| format!("wav {} is missing channel count", path.display()))?;
    let sample_rate = sample_rate.ok_or_else(|| format!("wav {} is missing sample rate", path.display()))?;
    if bits_per_sample != Some(16) {
        return Err(format!("wav {} must use 16-bit samples", path.display()));
    }
    let data = data.ok_or_else(|| format!("wav {} is missing a data chunk", path.display()))?;
    if data.len() % 2 != 0 {
        return Err(format!("wav {} data length must be even", path.display()));
    }

    let mut samples = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }

    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

fn compare_audio_metrics(samples: &[i16], sample_rate: u32, reference: &WavData) -> AudioCompareMetrics {
    let local = wav_first_channel(samples, 2);
    let reference_channel = wav_first_channel(&reference.samples, reference.channels);
    let len = local.len().min(reference_channel.len());
    let local = &local[..len];
    let reference_channel = &reference_channel[..len];

    let mut sum_sq = 0.0;
    let mut sum_x = 0.0;
    let mut sum_y = 0.0;
    let mut sum_xx = 0.0;
    let mut sum_yy = 0.0;
    let mut sum_xy = 0.0;
    let mut peak = 0i32;
    let mut reference_peak = 0i32;
    let mut first_nonzero_pair = len;
    let mut reference_first_nonzero_pair = len;

    for (idx, (&x, &y)) in local.iter().zip(reference_channel.iter()).enumerate() {
        let xf = f64::from(x);
        let yf = f64::from(y);
        let diff = xf - yf;
        sum_sq += diff * diff;
        sum_x += xf;
        sum_y += yf;
        sum_xx += xf * xf;
        sum_yy += yf * yf;
        sum_xy += xf * yf;
        peak = peak.max(i32::from(x).abs());
        reference_peak = reference_peak.max(i32::from(y).abs());
        if x != 0 && first_nonzero_pair == len {
            first_nonzero_pair = idx;
        }
        if y != 0 && reference_first_nonzero_pair == len {
            reference_first_nonzero_pair = idx;
        }
    }

    if reference.sample_rate != sample_rate {
        eprintln!(
            "warning: sample rate mismatch (local {} Hz, reference {} Hz)",
            sample_rate, reference.sample_rate
        );
    }

    let rmse = if len == 0 { 0.0 } else { (sum_sq / len as f64).sqrt() };
    let mean_x = if len == 0 { 0.0 } else { sum_x / len as f64 };
    let mean_y = if len == 0 { 0.0 } else { sum_y / len as f64 };
    let cov = sum_xy - len as f64 * mean_x * mean_y;
    let var_x = sum_xx - len as f64 * mean_x * mean_x;
    let var_y = sum_yy - len as f64 * mean_y * mean_y;
    let correlation = if var_x <= 0.0 || var_y <= 0.0 {
        if local == reference_channel { 1.0 } else { 0.0 }
    } else {
        cov / (var_x.sqrt() * var_y.sqrt())
    };
    let scale = if sum_xx == 0.0 { 0.0 } else { sum_xy / sum_xx };

    AudioCompareMetrics {
        rmse,
        correlation,
        scale,
        first_nonzero_pair,
        reference_first_nonzero_pair,
        peak,
        reference_peak,
    }
}

fn wav_first_channel(samples: &[i16], channels: u16) -> Vec<i16> {
    let step = usize::from(channels.max(1));
    samples.iter().step_by(step).copied().collect()
}
