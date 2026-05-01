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
        .ok_or_else(|| "usage: inspect <rom> <frames> [--replay file] [--dump-frame file.ppm] [--dump-audio file.wav]".to_string())?;
    let frames: u32 = args
        .next()
        .ok_or_else(|| "missing frame count".to_string())?
        .parse()
        .map_err(|_| "frame count must be an integer".to_string())?;

    let mut replay_path: Option<String> = None;
    let mut dump_frame_path: Option<String> = None;
    let mut dump_audio_path: Option<String> = None;
    let mut trace_frames = false;
    let mut step_count: u64 = 0;
    let mut trace_steps = false;
    let mut trace_until_steps = false;
    let mut until_pc: Option<u32> = None;
    let mut until_pc_hits: u64 = 1;
    let mut max_steps: u64 = 1_000_000;
    let mut peek_addrs: Vec<u32> = Vec::new();

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--replay" => replay_path = Some(args.next().ok_or_else(|| "missing replay path".to_string())?),
            "--dump-frame" => dump_frame_path = Some(args.next().ok_or_else(|| "missing frame path".to_string())?),
            "--dump-audio" => dump_audio_path = Some(args.next().ok_or_else(|| "missing audio path".to_string())?),
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
            _ => return Err(format!("unknown argument: {flag}")),
        }
    }

    let rom = fs::read(&rom_path).map_err(|e| format!("failed to read ROM: {e}"))?;
    let mut emu = NativeEmulator::new_with_rom(&rom).ok_or_else(|| "failed to initialize emulator".to_string())?;
    let replay = if let Some(path) = replay_path {
        load_replay(Path::new(&path))?
    } else {
        Vec::new()
    };

    let mut replay_index = 0usize;
    let mut current_keys = 0u32;
    let mut all_audio = Vec::new();

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

    println!("frames={frames}");
    println!("pc=0x{:08x}", emu.pc());
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
    let regs = emu.registers();
    for (idx, value) in regs.iter().enumerate() {
        println!("r{idx}=0x{value:08x}");
    }
    println!("fb_hash=0x{:016x}", fnv1a_u32(emu.framebuffer()));
    println!("audio_pairs={}", all_audio.len() / 2);
    println!("audio_rate={}", emu.audio_rate());
    println!("audio_hash=0x{:016x}", fnv1a_i16(&all_audio));
    for addr in peek_addrs {
        println!("peek32[0x{addr:08x}]=0x{:08x}", emu.peek_u32(addr));
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
