use std::env;
use std::fs;
use std::path::Path;

use gba_emu::NativeEmulator;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let rom_path = args
        .next()
        .ok_or_else(|| {
            "usage: inspect <rom> <frames> [--replay file] [--dump-frame file.ppm] [--dump-audio file.wav] [--dump-pair-input-audio file.wav] [--dump-prefilter-input-audio file.wav] [--dump-prefilter-audio file.wav] [--compare-audio file.wav] [--audio-delay-pairs n] [--audio-first-pair-cycles n] [--audio-prefilter-gain-num n] [--audio-params \"key=value ...\"] [--audio-capture average|midpoint|point:<num>/<den>|point-blend:<num>/<den>:<blend_num>/<blend_den>|point-point-blend:<num>/<den>:<other_num>/<other_den>:<blend_num>/<blend_den>|point-window-blend:<num>/<den>:<window_num>/<window_den>:<blend_num>/<blend_den>|point-segment-blend:<num>/<den>:<start_num>/<start_den>:<end_num>/<end_den>:<blend_num>/<blend_den>|point-segment-hybrid-blend:<num>/<den>:<start_num>/<start_den>:<end_num>/<end_den>:<shape_num>/<shape_den>:<blend_num>/<blend_den>|point-segment-late-linear-blend:<num>/<den>:<start_num>/<start_den>:<end_num>/<end_den>:<blend_num>/<blend_den>|point-segment-linear-blend:<num>/<den>:<start_num>/<start_den>:<end_num>/<end_den>:<blend_num>/<blend_den>|point-segment-ramp-blend:<num>/<den>:<start_num>/<start_den>:<end_num>/<end_den>:<blend_num>/<blend_den>|endpoint|endpoint-after-timer] [--audio-mix legacy|stereo-average|left|right] [--sound-dma-threshold n] [--sound-dma-prime fill|single|disabled] [--sound-dma-refill immediate|next-pop]"
                .to_string()
        })?;
    let frames: u32 = args
        .next()
        .ok_or_else(|| "missing frame count".to_string())?
        .parse()
        .map_err(|_| "frame count must be an integer".to_string())?;

    let mut replay_path: Option<String> = None;
    let mut dump_frame_path: Option<String> = None;
    let mut dump_audio_path: Option<String> = None;
    let mut dump_pair_input_audio_path: Option<String> = None;
    let mut dump_prefilter_input_audio_path: Option<String> = None;
    let mut dump_prefilter_audio_path: Option<String> = None;
    let mut compare_audio_path: Option<String> = None;
    let mut audio_delay_pairs_override: Option<usize> = None;
    let mut audio_first_pair_cycles_override: Option<u32> = None;
    let mut audio_prefilter_gain_num_override: Option<i32> = None;
    let mut audio_params_override: Option<String> = None;
    let mut audio_capture_mode_override: Option<String> = None;
    let mut audio_mix_mode_override: Option<String> = None;
    let mut sound_dma_threshold_override: Option<usize> = None;
    let mut sound_dma_prime_mode_override: Option<String> = None;
    let mut sound_dma_refill_mode_override: Option<String> = None;
    let mut trace_frames = false;
    let mut step_count: u64 = 0;
    let mut trace_steps = false;
    let mut trace_until_steps = false;
    let mut until_pc: Option<u32> = None;
    let mut until_pc_hits: u64 = 1;
    let mut max_steps: u64 = 1_000_000;
    let mut peek_addrs: Vec<u32> = Vec::new();
    let mut trace_sound = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--replay" => replay_path = Some(args.next().ok_or_else(|| "missing replay path".to_string())?),
            "--dump-frame" => dump_frame_path = Some(args.next().ok_or_else(|| "missing frame path".to_string())?),
            "--dump-audio" => dump_audio_path = Some(args.next().ok_or_else(|| "missing audio path".to_string())?),
            "--dump-pair-input-audio" => {
                dump_pair_input_audio_path = Some(args.next().ok_or_else(|| "missing pair-input audio path".to_string())?)
            }
            "--dump-prefilter-input-audio" => {
                dump_prefilter_input_audio_path =
                    Some(args.next().ok_or_else(|| "missing prefilter input audio path".to_string())?)
            }
            "--dump-prefilter-audio" => {
                dump_prefilter_audio_path = Some(args.next().ok_or_else(|| "missing prefilter audio path".to_string())?)
            }
            "--compare-audio" => {
                compare_audio_path = Some(args.next().ok_or_else(|| "missing comparison audio path".to_string())?)
            }
            "--audio-delay-pairs" => {
                audio_delay_pairs_override = Some(
                    args.next()
                        .ok_or_else(|| "missing audio delay pair count".to_string())?
                        .parse()
                        .map_err(|_| "audio delay pair count must be an integer".to_string())?,
                );
            }
            "--audio-first-pair-cycles" => {
                audio_first_pair_cycles_override = Some(
                    args.next()
                        .ok_or_else(|| "missing first-pair cycle count".to_string())?
                        .parse()
                        .map_err(|_| "first-pair cycle count must be an integer".to_string())?,
                );
            }
            "--audio-prefilter-gain-num" => {
                audio_prefilter_gain_num_override = Some(
                    args.next()
                        .ok_or_else(|| "missing prefilter gain numerator".to_string())?
                        .parse()
                        .map_err(|_| "prefilter gain numerator must be an integer".to_string())?,
                );
            }
            "--audio-params" => {
                audio_params_override = Some(args.next().ok_or_else(|| "missing audio params".to_string())?);
            }
            "--audio-capture" => {
                audio_capture_mode_override =
                    Some(args.next().ok_or_else(|| "missing audio capture mode".to_string())?);
            }
            "--audio-mix" => {
                audio_mix_mode_override = Some(args.next().ok_or_else(|| "missing audio mix mode".to_string())?);
            }
            "--sound-dma-threshold" => {
                sound_dma_threshold_override = Some(
                    args.next()
                        .ok_or_else(|| "missing sound DMA threshold".to_string())?
                        .parse()
                        .map_err(|_| "sound DMA threshold must be an integer".to_string())?,
                );
            }
            "--sound-dma-prime" => {
                sound_dma_prime_mode_override =
                    Some(args.next().ok_or_else(|| "missing sound DMA prime mode".to_string())?);
            }
            "--sound-dma-refill" => {
                sound_dma_refill_mode_override =
                    Some(args.next().ok_or_else(|| "missing sound DMA refill mode".to_string())?);
            }
            "--trace-frames" => trace_frames = true,
            "--trace-until-steps" => trace_until_steps = true,
            "--step" => {
                step_count = args
                    .next()
                    .ok_or_else(|| "missing step count".to_string())?
                    .parse()
                    .map_err(|_| "step count must be an integer".to_string())?;
            }
            "--trace-steps" => trace_steps = true,
            "--until-pc" => {
                let text = args.next().ok_or_else(|| "missing pc value".to_string())?;
                until_pc = Some(parse_u32(&text).map_err(|_| "invalid pc value".to_string())?);
            }
            "--until-pc-hits" => {
                until_pc_hits = args
                    .next()
                    .ok_or_else(|| "missing pc hit count".to_string())?
                    .parse()
                    .map_err(|_| "pc hit count must be an integer".to_string())?;
                if until_pc_hits == 0 {
                    return Err("pc hit count must be at least 1".to_string());
                }
            }
            "--max-steps" => {
                max_steps = args
                    .next()
                    .ok_or_else(|| "missing max step count".to_string())?
                    .parse()
                    .map_err(|_| "max step count must be an integer".to_string())?;
            }
            "--peek" => {
                let text = args.next().ok_or_else(|| "missing peek address".to_string())?;
                peek_addrs.push(parse_u32(&text).map_err(|_| "invalid peek address".to_string())?);
            }
            "--trace-sound" => trace_sound = true,
            _ => return Err(format!("unknown argument: {flag}")),
        }
    }

    let rom = fs::read(&rom_path).map_err(|e| format!("failed to read ROM: {e}"))?;
    let mut emu = NativeEmulator::new_with_rom(&rom).ok_or_else(|| "failed to initialize emulator".to_string())?;
    if let Some(delay_pairs) = audio_delay_pairs_override {
        emu.set_audio_delay_pairs_for_debug(delay_pairs);
    }
    if let Some(first_pair_cycles) = audio_first_pair_cycles_override {
        emu.set_audio_first_pair_cycles_for_debug(first_pair_cycles);
    }
    if let Some(prefilter_gain_num) = audio_prefilter_gain_num_override {
        emu.set_audio_prefilter_gain_num_for_debug(prefilter_gain_num);
    }
    if let Some(audio_params) = audio_params_override {
        emu.set_audio_params_for_debug(&audio_params)?;
    }
    if let Some(mode) = audio_capture_mode_override {
        emu.set_audio_capture_mode_for_debug(&mode)?;
    }
    if let Some(mode) = audio_mix_mode_override {
        emu.set_audio_mix_mode_for_debug(&mode)?;
    }
    if let Some(threshold) = sound_dma_threshold_override {
        emu.set_sound_fifo_dma_threshold_for_debug(threshold);
    }
    if let Some(mode) = sound_dma_prime_mode_override {
        emu.set_sound_fifo_prime_mode_for_debug(&mode)?;
    }
    if let Some(mode) = sound_dma_refill_mode_override {
        emu.set_sound_fifo_refill_mode_for_debug(&mode)?;
    }
    let replay = if let Some(path) = replay_path {
        load_replay(Path::new(&path))?
    } else {
        Vec::new()
    };

    let mut replay_index = 0usize;
    let mut current_keys = 0u32;
    let mut all_audio = Vec::new();
    let mut all_pair_input_audio = Vec::new();
    let mut all_prefilter_input_audio = Vec::new();
    let mut all_prefilter_audio = Vec::new();

    for frame in 0..frames {
        while replay_index < replay.len() && replay[replay_index].0 == frame {
            current_keys = replay[replay_index].1;
            replay_index += 1;
        }
        emu.set_keys(current_keys);
        emu.run_frame();
        if trace_frames {
            let regs = emu.registers();
            println!(
                "trace frame={} pc=0x{:08x} dispcnt=0x{:04x} r0=0x{:08x} r1=0x{:08x} lr=0x{:08x}",
                frame + 1,
                emu.pc(),
                emu.dispcnt(),
                regs[0],
                regs[1],
                regs[14]
            );
        }
        all_audio.extend(emu.take_audio());
        all_pair_input_audio.extend(emu.take_pair_input_audio());
        all_prefilter_input_audio.extend(emu.take_prefilter_input_audio());
        all_prefilter_audio.extend(emu.take_prefilter_audio());
    }

    if let Some(target_pc) = until_pc {
        let mut steps = 0u64;
        let mut hits = 0u64;
        while hits < until_pc_hits && steps < max_steps {
            let cycles = emu.step_instruction();
            if cycles == 0 {
                break;
            }
            steps += 1;
            if emu.pc() == target_pc {
                hits += 1;
            }
            if trace_until_steps {
                let regs = emu.registers();
                println!(
                    "step {} pc=0x{:08x} cycles={} frame={} frame_cycle={} r0=0x{:08x} r1=0x{:08x} lr=0x{:08x}",
                    steps,
                    emu.pc(),
                    cycles,
                    emu.frames_emulated(),
                    emu.frame_cycle(),
                    regs[0],
                    regs[1],
                    regs[14]
                );
            }
        }
        println!(
            "until_pc target=0x{:08x} hit={} hits={} steps={}",
            target_pc,
            emu.pc() == target_pc && hits >= until_pc_hits,
            hits,
            steps
        );
    }

    for step in 0..step_count {
        let cycles = emu.step_instruction();
        if cycles == 0 {
            break;
        }
        if trace_steps {
            let regs = emu.registers();
            println!(
                "step {} pc=0x{:08x} cycles={} frame={} frame_cycle={} r0=0x{:08x} r1=0x{:08x} lr=0x{:08x}",
                step + 1,
                emu.pc(),
                cycles,
                emu.frames_emulated(),
                emu.frame_cycle(),
                regs[0],
                regs[1],
                regs[14]
            );
        }
    }

    if let Some(path) = dump_frame_path {
        write_ppm(Path::new(&path), emu.framebuffer())?;
    }
    if let Some(path) = dump_audio_path {
        write_wav(Path::new(&path), &all_audio, emu.audio_rate() as u32)?;
    }
    if let Some(path) = dump_pair_input_audio_path {
        write_wav(Path::new(&path), &all_pair_input_audio, emu.audio_rate() as u32)?;
    }
    if let Some(path) = dump_prefilter_input_audio_path {
        write_wav(Path::new(&path), &all_prefilter_input_audio, emu.audio_rate() as u32)?;
    }
    if let Some(path) = dump_prefilter_audio_path {
        write_wav(Path::new(&path), &all_prefilter_audio, emu.audio_rate() as u32)?;
    }

    println!("frames={frames}");
    println!("pc=0x{:08x}", emu.pc());
    println!("cpsr=0x{:08x}", emu.cpsr());
    println!("thumb={}", emu.thumb());
    println!("halted={}", emu.halted());
    println!("stopped={}", emu.stopped());
    println!("dispcnt=0x{:04x}", emu.dispcnt());
    println!("vcount={}", emu.vcount());
    println!("instructions={}", emu.instruction_count());
    println!("dispcnt_writes={}", emu.dispcnt_writes());
    println!(
        "last_dispcnt_write=0x{:08x}:0x{:04x}",
        emu.last_dispcnt_write_pc(),
        emu.last_dispcnt_value()
    );
    println!(
        "last_ie_write=0x{:08x}:0x{:04x}",
        emu.last_ie_write_pc(),
        emu.last_ie_value()
    );
    println!(
        "last_haltcnt_write=0x{:08x}:0x{:02x}",
        emu.last_haltcnt_write_pc(),
        emu.last_haltcnt_value()
    );
    let regs = emu.registers();
    for (idx, value) in regs.iter().enumerate() {
        println!("r{idx}=0x{value:08x}");
    }
    println!("fb_hash=0x{:016x}", fnv1a_u32(emu.framebuffer()));
    println!("audio_pairs={}", all_audio.len() / 2);
    println!("audio_rate={}", emu.audio_rate());
    println!("audio_hash=0x{:016x}", fnv1a_i16(&all_audio));
    if let Some(path) = compare_audio_path {
        let reference = read_wav_i16(Path::new(&path))?;
        let metrics = compare_audio_metrics(&all_audio, emu.audio_rate() as u32, &reference);
        println!("compare_audio_channels={}", reference.channels);
        println!("compare_audio_rate={}", reference.sample_rate);
        println!("compare_audio_ref_pairs={}", wav_first_channel(&reference.samples, reference.channels).len());
        println!("compare_audio_rmse={:.6}", metrics.rmse);
        println!("compare_audio_corr={:.9}", metrics.correlation);
        println!("compare_audio_scale={:.9}", metrics.scale);
        println!(
            "compare_audio_first_nonzero_pair={} {}",
            metrics.first_nonzero_pair,
            metrics.reference_first_nonzero_pair
        );
        println!("compare_audio_peak={} {}", metrics.peak, metrics.reference_peak);
    }
    for addr in peek_addrs {
        println!("peek32[0x{addr:08x}]=0x{:08x}", emu.peek_u32(addr));
    }
    if trace_sound {
        println!("sound_dma_threshold={}", emu.sound_fifo_dma_threshold());
        println!("sound_dma_prime_mode={}", emu.sound_fifo_prime_mode());
        println!("sound_dma_refill_mode={}", emu.sound_fifo_refill_mode());
        println!("timer_overflow_total={:?}", emu.timer_overflow_total());
        println!("timer_overflow_max_batch={:?}", emu.timer_overflow_max_batch());
        println!("sound_pop_count={:?}", emu.sound_pop_count());
        println!("sound_dma_request_count={:?}", emu.sound_dma_request_count());
        println!("sound_dma_service_count={:?}", emu.sound_dma_service_count());
        println!("sound_dma_transfer_bytes={:?}", emu.sound_dma_transfer_bytes());
        println!("sound_a_sample={}", emu.direct_sound_a_sample());
        println!("sound_b_sample={}", emu.direct_sound_b_sample());
        println!("fifo_a_len={}", emu.fifo_a_len());
        println!("fifo_b_len={}", emu.fifo_b_len());
        println!("sound_a_writes={:?}", emu.debug_sound_a_writes());
        println!("sound_b_writes={:?}", emu.debug_sound_b_writes());
        println!("sound_a_pops={:?}", emu.debug_sound_a_pops());
        println!("sound_b_pops={:?}", emu.debug_sound_b_pops());
        println!("sound_a_cpu_writes={:?}", emu.debug_sound_a_cpu_writes());
        println!("sound_b_cpu_writes={:?}", emu.debug_sound_b_cpu_writes());
    }

    Ok(())
}

fn load_replay(path: &Path) -> Result<Vec<(u32, u32)>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("failed to read replay: {e}"))?;
    let mut replay = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let frame_text = parts
            .next()
            .ok_or_else(|| format!("replay line {} is missing frame", lineno + 1))?;
        let keys_text = parts
            .next()
            .ok_or_else(|| format!("replay line {} is missing keys", lineno + 1))?;
        if parts.next().is_some() {
            return Err(format!("replay line {} has extra data", lineno + 1));
        }
        let frame: u32 = frame_text
            .parse()
            .map_err(|_| format!("replay line {} has invalid frame", lineno + 1))?;
        let keys = parse_u32(keys_text)
            .map_err(|_| format!("replay line {} has invalid key mask", lineno + 1))?;
        replay.push((frame, keys));
    }
    replay.sort_by_key(|entry| entry.0);
    Ok(replay)
}

fn parse_u32(text: &str) -> Result<u32, std::num::ParseIntError> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse()
    }
}

fn write_ppm(path: &Path, framebuffer: &[u32]) -> Result<(), String> {
    let mut out = Vec::with_capacity(32 + framebuffer.len() * 3);
    out.extend_from_slice(b"P6\n240 160\n255\n");
    for &pixel in framebuffer {
        out.push((pixel & 0xff) as u8);
        out.push(((pixel >> 8) & 0xff) as u8);
        out.push(((pixel >> 16) & 0xff) as u8);
    }
    fs::write(path, out).map_err(|e| format!("failed to write frame dump: {e}"))
}

fn write_wav(path: &Path, samples: &[i16], sample_rate: u32) -> Result<(), String> {
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36 + data_bytes;
    let block_align = 4u16;
    let byte_rate = sample_rate * u32::from(block_align);

    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, out).map_err(|e| format!("failed to write audio dump: {e}"))
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

fn read_wav_i16(path: &Path) -> Result<WavData, String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read wav: {e}"))?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("wav must be RIFF/WAVE".to_string());
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
            return Err("wav chunk extends past end of file".to_string());
        }
        let chunk = &bytes[offset..offset + chunk_size];
        match chunk_id {
            b"fmt " => {
                if chunk.len() < 16 {
                    return Err("wav fmt chunk is too small".to_string());
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
        return Err("wav must be PCM".to_string());
    }
    let channels = channels.ok_or_else(|| "wav is missing channel count".to_string())?;
    let sample_rate = sample_rate.ok_or_else(|| "wav is missing sample rate".to_string())?;
    if bits_per_sample != Some(16) {
        return Err("wav must use 16-bit samples".to_string());
    }
    let data = data.ok_or_else(|| "wav is missing a data chunk".to_string())?;
    if data.len() % 2 != 0 {
        return Err("wav data length must be even".to_string());
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

fn fnv1a_u32(values: &[u32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &value in values {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }
    hash
}

fn fnv1a_i16(values: &[i16]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &value in values {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }
    hash
}
