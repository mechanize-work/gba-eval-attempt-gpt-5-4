mod cpu;
mod ppu;

use std::collections::VecDeque;
use std::cell::UnsafeCell;
use std::sync::OnceLock;

use cpu::{Cpu, Mode};

const BIOS_SIZE: usize = 16 * 1024;
const ROM_BUFFER_SIZE: usize = 32 * 1024 * 1024;
const EWRAM_SIZE: usize = 256 * 1024;
const IWRAM_SIZE: usize = 32 * 1024;
const IO_SIZE: usize = 0x400;
const PALETTE_SIZE: usize = 1024;
const VRAM_SIZE: usize = 96 * 1024;
const OAM_SIZE: usize = 1024;
const SRAM_SIZE: usize = 64 * 1024;

const SCREEN_WIDTH: usize = 240;
const SCREEN_HEIGHT: usize = 160;

const CPU_CLOCK_HZ: u32 = 16_777_216;
const DEFAULT_AUDIO_RATE: u32 = 32_768;
const DOUBLE_AUDIO_RATE: u32 = 65_536;
const BOOT_AUDIO_PREROLL_PAIRS: usize = 1_500;
const INITIAL_AUDIO_FIRST_PAIR_CYCLES: u32 = 279;
const INITIAL_AUDIO_FRACTION: u64 =
    CPU_CLOCK_HZ as u64 - DEFAULT_AUDIO_RATE as u64 * INITIAL_AUDIO_FIRST_PAIR_CYCLES as u64;

const CYCLES_PER_LINE: u32 = 1_232;
const HDRAW_CYCLES: u32 = 1_006;
const VISIBLE_LINES: u32 = 160;
const TOTAL_LINES: u32 = 228;
const FRAME_CYCLES: u32 = CYCLES_PER_LINE * TOTAL_LINES;

const REG_DISPCNT: usize = 0x000;
const REG_DISPSTAT: usize = 0x004;
const REG_VCOUNT: usize = 0x006;
const REG_SOUNDCNT_H: usize = 0x082;
const REG_SOUNDCNT_X: usize = 0x084;
const REG_SOUNDBIAS: usize = 0x088;
const REG_FIFO_A: usize = 0x0a0;
const REG_FIFO_B: usize = 0x0a4;
const REG_TM0CNT_L: usize = 0x100;
#[cfg(test)]
const REG_TM0CNT_H: usize = 0x102;
const REG_KEYINPUT: usize = 0x130;
const REG_KEYCNT: usize = 0x132;
const REG_IE: usize = 0x200;
const REG_IF: usize = 0x202;
const REG_WAITCNT: usize = 0x204;
const REG_IME: usize = 0x208;
const REG_HALTCNT: usize = 0x301;

const IRQ_VBLANK: u16 = 1 << 0;
const IRQ_HBLANK: u16 = 1 << 1;
const IRQ_VCOUNT: u16 = 1 << 2;
const IRQ_TIMER0: u16 = 1 << 3;
const IRQ_TIMER1: u16 = 1 << 4;
const IRQ_TIMER2: u16 = 1 << 5;
const IRQ_TIMER3: u16 = 1 << 6;

const DMA_REG_BASES: [usize; 4] = [0x0b0, 0x0bc, 0x0c8, 0x0d4];
const TIMER_REG_BASES: [usize; 4] = [REG_TM0CNT_L, 0x104, 0x108, 0x10c];
const TIMER_IRQS: [u16; 4] = [IRQ_TIMER0, IRQ_TIMER1, IRQ_TIMER2, IRQ_TIMER3];
const TIMER_PRESCALERS: [u32; 4] = [1, 64, 256, 1_024];
const DIRECT_SOUND_FIFO_CAPACITY: usize = 32;
const DIRECT_SOUND_FIFO_DMA_THRESHOLD: usize = 16;
const AUDIO_OUTPUT_SCALE: i32 = 64;
const AUDIO_OUTPUT_DELAY_PAIRS: usize = 165;
const AUDIO_OUTPUT_GAIN_NUM: i32 = 1;
const AUDIO_OUTPUT_GAIN_DEN: i32 = 4;
const AUDIO_OUTPUT_INPUT_FILTER_CUR: i32 = 136;
const AUDIO_OUTPUT_INPUT_FILTER_PREV: i32 = -16;
const AUDIO_OUTPUT_INPUT_FILTER_DEN: i32 = 128;
const AUDIO_OUTPUT_PREFILTER_GAIN_NUM: i32 = 136;
const AUDIO_OUTPUT_PREFILTER_GAIN_DEN: i32 = 128;
const AUDIO_OUTPUT_FILTER_TAPS: [i32; 8] = [49, 11, -3, 18, -15, 3, 7, -6];
const AUDIO_OUTPUT_FILTER_DEN: i32 = 64;
const AUDIO_OUTPUT_DEADZONE: i32 = 0;
const AUDIO_OUTPUT_POST_GAIN_NUM: i32 = 127;
const AUDIO_OUTPUT_POST_GAIN_DEN: i32 = 128;
const AUDIO_OUTPUT_COMPRESS_THRESHOLD_POSITIVE: i32 = 2_400;
const AUDIO_OUTPUT_COMPRESS_THRESHOLD_NEGATIVE: i32 = 2_080;
const AUDIO_OUTPUT_COMPRESS_NUM_POSITIVE: i32 = 128;
const AUDIO_OUTPUT_COMPRESS_NUM_NEGATIVE: i32 = 127;
const AUDIO_OUTPUT_COMPRESS_DEN: i32 = 128;
const AUDIO_OUTPUT_POSITIVE_BIAS: i32 = 66;
const AUDIO_OUTPUT_NEGATIVE_BIAS: i32 = 71;
const AUDIO_OUTPUT_POST_FILTER_CUR: i32 = 136;
const AUDIO_OUTPUT_POST_FILTER_PREV: i32 = -8;
const AUDIO_OUTPUT_POST_FILTER_PREV2: i32 = 6;
const AUDIO_OUTPUT_POST_FILTER_DEN: i32 = 128;
const AUDIO_OUTPUT_POST_FILTER_POSITIVE_BIAS: i32 = 5;
const AUDIO_OUTPUT_POST_FILTER_NEGATIVE_BIAS: i32 = -3;
const AUDIO_OUTPUT_SIGN_HYSTERESIS: i32 = 28;
const AUDIO_OUTPUT_FINAL_FILTER_TAPS: [i32; 4] = [126, 0, 4, -8];
const AUDIO_OUTPUT_FINAL_FILTER_DEN: i32 = 128;
const AUDIO_OUTPUT_FINAL_NONZERO_BIAS: i32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DmaTiming {
    Immediate,
    VBlank,
    HBlank,
    Special,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AudioCaptureMode {
    Average,
    Endpoint,
}

#[derive(Clone, Copy, Debug)]
struct AudioOutputParams {
    deadzone: i32,
    input_filter_cur: i32,
    input_filter_prev: i32,
    prefilter_gain_num: i32,
    compress_threshold_positive: i32,
    compress_threshold_negative: i32,
    compress_num_positive: i32,
    compress_num_negative: i32,
    positive_bias: i32,
    negative_bias: i32,
    post_filter_cur: i32,
    post_filter_prev: i32,
    post_filter_prev2: i32,
    post_filter_positive_bias: i32,
    post_filter_negative_bias: i32,
    sign_hysteresis: i32,
    final_filter_taps: [i32; 4],
    final_nonzero_bias: i32,
}

impl Default for AudioOutputParams {
    fn default() -> Self {
        Self {
            deadzone: AUDIO_OUTPUT_DEADZONE,
            input_filter_cur: AUDIO_OUTPUT_INPUT_FILTER_CUR,
            input_filter_prev: AUDIO_OUTPUT_INPUT_FILTER_PREV,
            prefilter_gain_num: AUDIO_OUTPUT_PREFILTER_GAIN_NUM,
            compress_threshold_positive: AUDIO_OUTPUT_COMPRESS_THRESHOLD_POSITIVE,
            compress_threshold_negative: AUDIO_OUTPUT_COMPRESS_THRESHOLD_NEGATIVE,
            compress_num_positive: AUDIO_OUTPUT_COMPRESS_NUM_POSITIVE,
            compress_num_negative: AUDIO_OUTPUT_COMPRESS_NUM_NEGATIVE,
            positive_bias: AUDIO_OUTPUT_POSITIVE_BIAS,
            negative_bias: AUDIO_OUTPUT_NEGATIVE_BIAS,
            post_filter_cur: AUDIO_OUTPUT_POST_FILTER_CUR,
            post_filter_prev: AUDIO_OUTPUT_POST_FILTER_PREV,
            post_filter_prev2: AUDIO_OUTPUT_POST_FILTER_PREV2,
            post_filter_positive_bias: AUDIO_OUTPUT_POST_FILTER_POSITIVE_BIAS,
            post_filter_negative_bias: AUDIO_OUTPUT_POST_FILTER_NEGATIVE_BIAS,
            sign_hysteresis: AUDIO_OUTPUT_SIGN_HYSTERESIS,
            final_filter_taps: AUDIO_OUTPUT_FINAL_FILTER_TAPS,
            final_nonzero_bias: AUDIO_OUTPUT_FINAL_NONZERO_BIAS,
        }
    }
}

struct GlobalEmulator(UnsafeCell<Emulator>);

unsafe impl Sync for GlobalEmulator {}

static EMULATOR: OnceLock<GlobalEmulator> = OnceLock::new();

pub(crate) struct Emulator {
    bios: [u8; BIOS_SIZE],
    rom_staging: Vec<u8>,
    rom_len: usize,
    ewram: Vec<u8>,
    iwram: Vec<u8>,
    io: [u8; IO_SIZE],
    palette: Vec<u8>,
    vram: Vec<u8>,
    oam: Vec<u8>,
    sram: Vec<u8>,
    cpu: Cpu,
    framebuffer: Vec<u32>,
    audio_buffer: Vec<i16>,
    audio_pair_input_buffer: Vec<i16>,
    audio_prefilter_input_buffer: Vec<i16>,
    audio_prefilter_buffer: Vec<i16>,
    audio_capture_mode: AudioCaptureMode,
    audio_output_params: AudioOutputParams,
    initial_audio_fraction: u64,
    audio_fraction: u64,
    audio_accum_left: i64,
    audio_accum_right: i64,
    audio_accum_cycles: u32,
    audio_delay_pairs: usize,
    audio_delay_line: VecDeque<i32>,
    audio_input_history: i16,
    audio_filter_history: [i32; AUDIO_OUTPUT_FILTER_TAPS.len() - 1],
    audio_post_history: i16,
    audio_post_history2: i16,
    audio_last_nonzero_output: i16,
    audio_final_filter_history: [i16; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1],
    active_dma_channel: Option<usize>,
    fifo_a: VecDeque<i8>,
    fifo_b: VecDeque<i8>,
    direct_sound_a_sample: i8,
    direct_sound_b_sample: i8,
    keys: u16,
    frame_cycle: u32,
    dma_src: [u32; 4],
    dma_dst: [u32; 4],
    timer_counter: [u16; 4],
    timer_remainder: [u32; 4],
    halted: bool,
    stopped: bool,
    frames_emulated: u64,
    debug_instruction_count: u64,
    debug_dispcnt_writes: u32,
    debug_last_dispcnt_write_pc: u32,
    debug_last_dispcnt_value: u16,
    debug_last_ie_write_pc: u32,
    debug_last_ie_value: u16,
    debug_last_haltcnt_write_pc: u32,
    debug_last_haltcnt_value: u8,
    debug_sound_a_writes: Vec<i8>,
    debug_sound_b_writes: Vec<i8>,
    debug_sound_a_pops: Vec<i8>,
    debug_sound_b_pops: Vec<i8>,
    debug_sound_a_cpu_writes: Vec<i8>,
    debug_sound_b_cpu_writes: Vec<i8>,
}

impl Emulator {
    fn new() -> Self {
        let mut bios = [0_u8; BIOS_SIZE];
        bios.copy_from_slice(include_bytes!("../spec/gba_bios_stub.bin"));

        let mut emu = Self {
            bios,
            rom_staging: vec![0; ROM_BUFFER_SIZE],
            rom_len: 0,
            ewram: vec![0; EWRAM_SIZE],
            iwram: vec![0; IWRAM_SIZE],
            io: [0; IO_SIZE],
            palette: vec![0; PALETTE_SIZE],
            vram: vec![0; VRAM_SIZE],
            oam: vec![0; OAM_SIZE],
            sram: vec![0; SRAM_SIZE],
            cpu: Cpu::new(),
            framebuffer: vec![0xff00_0000; SCREEN_WIDTH * SCREEN_HEIGHT],
            audio_buffer: Vec::with_capacity(4_096),
            audio_pair_input_buffer: Vec::with_capacity(4_096),
            audio_prefilter_input_buffer: Vec::with_capacity(4_096),
            audio_prefilter_buffer: Vec::with_capacity(4_096),
            audio_capture_mode: AudioCaptureMode::Average,
            audio_output_params: AudioOutputParams::default(),
            initial_audio_fraction: INITIAL_AUDIO_FRACTION,
            audio_fraction: INITIAL_AUDIO_FRACTION,
            audio_accum_left: 0,
            audio_accum_right: 0,
            audio_accum_cycles: 0,
            audio_delay_pairs: AUDIO_OUTPUT_DELAY_PAIRS,
            audio_delay_line: VecDeque::with_capacity(AUDIO_OUTPUT_DELAY_PAIRS + 1),
            audio_input_history: 0,
            audio_filter_history: [0; AUDIO_OUTPUT_FILTER_TAPS.len() - 1],
            audio_post_history: 0,
            audio_post_history2: 0,
            audio_last_nonzero_output: 0,
            audio_final_filter_history: [0; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1],
            active_dma_channel: None,
            fifo_a: VecDeque::with_capacity(DIRECT_SOUND_FIFO_CAPACITY),
            fifo_b: VecDeque::with_capacity(DIRECT_SOUND_FIFO_CAPACITY),
            direct_sound_a_sample: 0,
            direct_sound_b_sample: 0,
            keys: 0,
            frame_cycle: 0,
            dma_src: [0; 4],
            dma_dst: [0; 4],
            timer_counter: [0; 4],
            timer_remainder: [0; 4],
            halted: false,
            stopped: false,
            frames_emulated: 0,
            debug_instruction_count: 0,
            debug_dispcnt_writes: 0,
            debug_last_dispcnt_write_pc: 0,
            debug_last_dispcnt_value: 0,
            debug_last_ie_write_pc: 0,
            debug_last_ie_value: 0,
            debug_last_haltcnt_write_pc: 0,
            debug_last_haltcnt_value: 0,
            debug_sound_a_writes: Vec::with_capacity(64),
            debug_sound_b_writes: Vec::with_capacity(64),
            debug_sound_a_pops: Vec::with_capacity(64),
            debug_sound_b_pops: Vec::with_capacity(64),
            debug_sound_a_cpu_writes: Vec::with_capacity(16),
            debug_sound_b_cpu_writes: Vec::with_capacity(16),
        };
        emu.reset_runtime_state();
        emu
    }

    fn load_rom(&mut self, len: usize) -> bool {
        if len == 0 || len > self.rom_staging.len() {
            return false;
        }
        self.rom_len = len;
        self.reset_runtime_state();
        true
    }

    fn reset_runtime_state(&mut self) {
        self.ewram.fill(0);
        self.iwram.fill(0);
        self.io.fill(0);
        self.palette.fill(0);
        self.vram.fill(0);
        self.oam.fill(0);
        self.sram.fill(0);
        self.framebuffer.fill(0xff00_0000);
        self.audio_buffer.clear();
        self.audio_pair_input_buffer.clear();
        self.audio_prefilter_input_buffer.clear();
        self.audio_prefilter_buffer.clear();
        self.audio_fraction = self.initial_audio_fraction;
        self.audio_accum_left = 0;
        self.audio_accum_right = 0;
        self.audio_accum_cycles = 0;
        self.audio_delay_line.clear();
        self.audio_input_history = 0;
        self.audio_filter_history.fill(0);
        self.audio_post_history = 0;
        self.audio_post_history2 = 0;
        self.audio_last_nonzero_output = 0;
        self.audio_final_filter_history.fill(0);
        self.active_dma_channel = None;
        self.fifo_a.clear();
        self.fifo_b.clear();
        self.direct_sound_a_sample = 0;
        self.direct_sound_b_sample = 0;
        self.keys = 0;
        self.frame_cycle = 0;
        self.dma_src = [0; 4];
        self.dma_dst = [0; 4];
        self.timer_counter = [0; 4];
        self.timer_remainder = [0; 4];
        self.halted = false;
        self.stopped = false;
        self.frames_emulated = 0;
        self.debug_instruction_count = 0;
        self.debug_dispcnt_writes = 0;
        self.debug_last_dispcnt_write_pc = 0;
        self.debug_last_dispcnt_value = 0;
        self.debug_last_ie_write_pc = 0;
        self.debug_last_ie_value = 0;
        self.debug_last_haltcnt_write_pc = 0;
        self.debug_last_haltcnt_value = 0;
        self.debug_sound_a_writes.clear();
        self.debug_sound_b_writes.clear();
        self.debug_sound_a_pops.clear();
        self.debug_sound_b_pops.clear();
        self.debug_sound_a_cpu_writes.clear();
        self.debug_sound_b_cpu_writes.clear();
        self.cpu.reset();

        self.io_write_u16_raw(REG_DISPCNT, 0x0080);
        self.io_write_u16_raw(REG_KEYCNT, 0);
        self.io_write_u16_raw(REG_IE, 0);
        self.io_write_u16_raw(REG_IF, 0);
        self.io_write_u16_raw(REG_WAITCNT, 0);
        self.io_write_u16_raw(REG_DISPSTAT, 0);
        self.io_write_u16_raw(REG_SOUNDBIAS, 0x0200);
        self.io_write_u8_raw(REG_IME, 0);
        self.io_write_u8_raw(REG_HALTCNT, 0);
    }

    fn run_frame(&mut self) {
        self.append_boot_audio_preroll();
        self.run_cycles(FRAME_CYCLES);
        self.frames_emulated = self.frames_emulated.wrapping_add(1);
    }

    fn run_cycles(&mut self, mut cycles: u32) {
        while cycles > 0 {
            if self.irq_pending() {
                let used = self.service_irq();
                self.advance_time(used);
                cycles = cycles.saturating_sub(used);
                continue;
            }

            if self.stopped {
                let used = self.advance_time_internal(cycles, true);
                cycles = cycles.saturating_sub(used);
                continue;
            }

            if self.halted {
                let used = self.advance_time_internal(cycles, true);
                cycles = cycles.saturating_sub(used);
                continue;
            }

            let used = self.step_cpu().max(1);
            self.advance_time(used);
            cycles = cycles.saturating_sub(used);
        }
    }

    fn irq_pending(&self) -> bool {
        (self.io[REG_IME] & 1) != 0
            && !self.cpu.irq_disabled()
            && (self.io_read_u16_raw(REG_IE) & self.io_read_u16_raw(REG_IF)) != 0
    }

    fn service_irq(&mut self) -> u32 {
        self.cpu
            .enter_exception(Mode::Irq, 0x18, self.cpu.pc().wrapping_add(4));
        3
    }

    fn audio_rate(&self) -> i32 {
        let sound_bias = self.io_read_u16_raw(REG_SOUNDBIAS);
        if (sound_bias >> 14) & 0x3 != 0 {
            DOUBLE_AUDIO_RATE as i32
        } else {
            DEFAULT_AUDIO_RATE as i32
        }
    }

    fn append_boot_audio_preroll(&mut self) {
        if self.frames_emulated != 0 {
            return;
        }
        let preroll_pairs = (BOOT_AUDIO_PREROLL_PAIRS as u64 * self.audio_rate() as u64 / DEFAULT_AUDIO_RATE as u64)
            as usize;
        self.audio_buffer
            .resize(self.audio_buffer.len() + preroll_pairs * 2, 0);
        self.audio_pair_input_buffer
            .resize(self.audio_pair_input_buffer.len() + preroll_pairs * 2, 0);
        self.audio_prefilter_input_buffer
            .resize(self.audio_prefilter_input_buffer.len() + preroll_pairs * 2, 0);
        self.audio_prefilter_buffer
            .resize(self.audio_prefilter_buffer.len() + preroll_pairs * 2, 0);
    }

    fn cycles_to_next_audio_pair(&self) -> u32 {
        let rate = self.audio_rate() as u64;
        let remaining_fraction = CPU_CLOCK_HZ as u64 - self.audio_fraction;
        ((remaining_fraction + rate - 1) / rate).max(1) as u32
    }

    fn emit_audio_for_cycles(&mut self, cycles: u32) {
        if cycles == 0 {
            return;
        }
        let (left, right) = self.mix_audio_level();
        self.audio_accum_left += i64::from(left) * i64::from(cycles);
        self.audio_accum_right += i64::from(right) * i64::from(cycles);
        self.audio_accum_cycles += cycles;

        let rate = self.audio_rate() as u64;
        self.audio_fraction += rate * cycles as u64;
        let pairs = (self.audio_fraction / CPU_CLOCK_HZ as u64) as usize;
        self.audio_fraction %= CPU_CLOCK_HZ as u64;
        if pairs == 0 {
            return;
        }
        let (avg_left, avg_right) = match self.audio_capture_mode {
            AudioCaptureMode::Average => (
                (self.audio_accum_left / i64::from(self.audio_accum_cycles))
                    .clamp(i16::MIN as i64, i16::MAX as i64) as i16,
                (self.audio_accum_right / i64::from(self.audio_accum_cycles))
                    .clamp(i16::MIN as i64, i16::MAX as i64) as i16,
            ),
            AudioCaptureMode::Endpoint => (
                left.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                right.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            ),
        };
        let pair_input = ((i32::from(avg_left) + i32::from(avg_right)) / 2) as i16;
        for _ in 0..pairs {
            let (prefilter_input, prefilter, output) = self.filter_audio_output(pair_input);
            self.audio_pair_input_buffer.push(pair_input);
            self.audio_pair_input_buffer.push(pair_input);
            self.audio_prefilter_input_buffer.push(prefilter_input);
            self.audio_prefilter_input_buffer.push(prefilter_input);
            self.audio_prefilter_buffer.push(prefilter);
            self.audio_prefilter_buffer.push(prefilter);
            self.audio_buffer.push(output);
            self.audio_buffer.push(output);
        }
        self.audio_accum_left = 0;
        self.audio_accum_right = 0;
        self.audio_accum_cycles = 0;
    }

    fn filter_audio_output(&mut self, sample: i16) -> (i16, i16, i16) {
        let params = self.audio_output_params;
        self.audio_delay_line.push_back(i32::from(sample));
        let delayed = if self.audio_delay_line.len() > self.audio_delay_pairs {
            self.audio_delay_line.pop_front().unwrap_or(0)
        } else {
            0
        };
        let prefilter_input = delayed * AUDIO_OUTPUT_GAIN_NUM / AUDIO_OUTPUT_GAIN_DEN;
        let filtered_input = Self::round_divide(
            prefilter_input * params.input_filter_cur
                + i32::from(self.audio_input_history) * params.input_filter_prev,
            AUDIO_OUTPUT_INPUT_FILTER_DEN,
        )
        .clamp(i16::MIN as i32, i16::MAX as i32);
        self.audio_input_history = prefilter_input.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let scaled = Self::round_divide(filtered_input * params.prefilter_gain_num, AUDIO_OUTPUT_PREFILTER_GAIN_DEN);
        let mut accum = AUDIO_OUTPUT_FILTER_TAPS[0] * scaled;
        for (tap, history) in AUDIO_OUTPUT_FILTER_TAPS[1..].iter().zip(self.audio_filter_history.iter()) {
            accum += *tap * *history;
        }
        let filtered = Self::round_divide(accum, AUDIO_OUTPUT_FILTER_DEN);
        let history_len = self.audio_filter_history.len();
        self.audio_filter_history.copy_within(0..history_len - 1, 1);
        self.audio_filter_history[0] = scaled;
        let clamped = filtered.clamp(i16::MIN as i32, i16::MAX as i32);
        let raw_output = if clamped.abs() <= params.deadzone {
            0
        } else {
            let gained = Self::round_divide(clamped * AUDIO_OUTPUT_POST_GAIN_NUM, AUDIO_OUTPUT_POST_GAIN_DEN)
                .clamp(i16::MIN as i32, i16::MAX as i32);
            let compressed = if gained > params.compress_threshold_positive {
                let above = gained - params.compress_threshold_positive;
                params.compress_threshold_positive
                    + Self::round_divide(above * params.compress_num_positive, AUDIO_OUTPUT_COMPRESS_DEN)
            } else if gained < -params.compress_threshold_negative {
                let above = (-gained) - params.compress_threshold_negative;
                -(params.compress_threshold_negative
                    + Self::round_divide(above * params.compress_num_negative, AUDIO_OUTPUT_COMPRESS_DEN))
            } else {
                gained
            };
            let biased = if compressed > 0 {
                compressed + params.positive_bias
            } else {
                compressed + params.negative_bias
            };
            let post_filtered = Self::round_divide(
                biased * params.post_filter_cur
                    + i32::from(self.audio_post_history) * params.post_filter_prev
                    + i32::from(self.audio_post_history2) * params.post_filter_prev2,
                AUDIO_OUTPUT_POST_FILTER_DEN,
            )
            .clamp(i16::MIN as i32, i16::MAX as i32);
            self.audio_post_history2 = self.audio_post_history;
            self.audio_post_history = biased.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let mut output = if post_filtered == 0 {
                0
            } else if post_filtered > 0 {
                (post_filtered + params.post_filter_positive_bias)
                    .clamp(i16::MIN as i32, i16::MAX as i32) as i16
            } else {
                (post_filtered + params.post_filter_negative_bias)
                    .clamp(i16::MIN as i32, i16::MAX as i32) as i16
            };
            if self.audio_last_nonzero_output != 0
                && output != 0
                && (self.audio_last_nonzero_output > 0) != (output > 0)
                && i32::from(output).abs() <= params.sign_hysteresis
            {
                output = 0;
            }
            if output != 0 {
                self.audio_last_nonzero_output = output;
            }
            output
        };
        let mut final_accum = params.final_filter_taps[0] * i32::from(raw_output);
        for (tap, history) in params.final_filter_taps[1..].iter().zip(self.audio_final_filter_history.iter()) {
            final_accum += *tap * i32::from(*history);
        }
        let corrected_output =
            Self::round_divide(final_accum, AUDIO_OUTPUT_FINAL_FILTER_DEN).clamp(i16::MIN as i32, i16::MAX as i32)
                as i16;
        let corrected_output = if corrected_output == 0 {
            0
        } else {
            (i32::from(corrected_output) + params.final_nonzero_bias)
                .clamp(i16::MIN as i32, i16::MAX as i32) as i16
        };
        let final_history_len = self.audio_final_filter_history.len();
        self.audio_final_filter_history
            .copy_within(0..final_history_len - 1, 1);
        self.audio_final_filter_history[0] = raw_output;
        (
            prefilter_input.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            scaled.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            corrected_output,
        )
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn reset_audio_output_history_for_debug(&mut self) {
        self.audio_buffer.clear();
        self.audio_pair_input_buffer.clear();
        self.audio_prefilter_input_buffer.clear();
        self.audio_prefilter_buffer.clear();
        self.audio_accum_left = 0;
        self.audio_accum_right = 0;
        self.audio_accum_cycles = 0;
        self.audio_delay_line.clear();
        self.audio_input_history = 0;
        self.audio_filter_history = [0; AUDIO_OUTPUT_FILTER_TAPS.len() - 1];
        self.audio_post_history = 0;
        self.audio_post_history2 = 0;
        self.audio_last_nonzero_output = 0;
        self.audio_final_filter_history = [0; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1];
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn set_audio_params_for_debug(&mut self, spec: &str) -> Result<(), String> {
        let mut params = self.audio_output_params;
        for token in spec.split(|ch: char| ch == ',' || ch.is_whitespace()) {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let Some((key, value_text)) = token.split_once('=') else {
                continue;
            };
            if matches!(key, "total_rmse" | "objective_total" | "total_peak_overage") {
                continue;
            }
            let value = value_text
                .parse::<i32>()
                .map_err(|_| format!("invalid audio param value for {key}: {value_text}"))?;
            match key {
                "dead" => params.deadzone = value.max(0),
                "icur" => params.input_filter_cur = value.clamp(120, 144),
                "iprev" => params.input_filter_prev = value.clamp(-24, 24),
                "pregain" => params.prefilter_gain_num = value.clamp(120, 144),
                "pthr" => params.compress_threshold_positive = value.max(0),
                "nthr" => params.compress_threshold_negative = value.max(0),
                "pcnum" => params.compress_num_positive = value.clamp(120, 128),
                "ncnum" => params.compress_num_negative = value.clamp(120, 128),
                "pos_bias" => params.positive_bias = value,
                "neg_bias" => params.negative_bias = value,
                "cur" => params.post_filter_cur = value.clamp(120, 136),
                "prev" => params.post_filter_prev = value.clamp(-8, 8),
                "prev2" => params.post_filter_prev2 = value.clamp(-8, 8),
                "post_pos_bias" => params.post_filter_positive_bias = value,
                "post_neg_bias" => params.post_filter_negative_bias = value,
                "sign_hyst" => params.sign_hysteresis = value.max(0),
                "fcur" => params.final_filter_taps[0] = value.clamp(120, 128),
                "fprev" => params.final_filter_taps[1] = value.clamp(0, 16),
                "fprev2" => params.final_filter_taps[2] = value.clamp(-8, 4),
                "fprev3" => params.final_filter_taps[3] = value.clamp(-8, 4),
                "fnonzero" => params.final_nonzero_bias = value,
                _ => return Err(format!("unknown audio param key: {key}")),
            }
        }
        self.audio_output_params = params;
        self.reset_audio_output_history_for_debug();
        Ok(())
    }

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    fn audio_params_for_debug(&self) -> String {
        format!(
            "dead={} icur={} iprev={} pregain={} pthr={} nthr={} pcnum={} ncnum={} pos_bias={} neg_bias={} cur={} prev={} prev2={} post_pos_bias={} post_neg_bias={} sign_hyst={} fcur={} fprev={} fprev2={} fprev3={} fnonzero={}",
            self.audio_output_params.deadzone,
            self.audio_output_params.input_filter_cur,
            self.audio_output_params.input_filter_prev,
            self.audio_output_params.prefilter_gain_num,
            self.audio_output_params.compress_threshold_positive,
            self.audio_output_params.compress_threshold_negative,
            self.audio_output_params.compress_num_positive,
            self.audio_output_params.compress_num_negative,
            self.audio_output_params.positive_bias,
            self.audio_output_params.negative_bias,
            self.audio_output_params.post_filter_cur,
            self.audio_output_params.post_filter_prev,
            self.audio_output_params.post_filter_prev2,
            self.audio_output_params.post_filter_positive_bias,
            self.audio_output_params.post_filter_negative_bias,
            self.audio_output_params.sign_hysteresis,
            self.audio_output_params.final_filter_taps[0],
            self.audio_output_params.final_filter_taps[1],
            self.audio_output_params.final_filter_taps[2],
            self.audio_output_params.final_filter_taps[3],
            self.audio_output_params.final_nonzero_bias,
        )
    }

    fn round_divide(value: i32, denominator: i32) -> i32 {
        if value >= 0 {
            (value + denominator / 2) / denominator
        } else {
            -((-value + denominator / 2) / denominator)
        }
    }

    fn mix_audio_level(&self) -> (i32, i32) {
        if self.io[REG_SOUNDCNT_X] & 0x80 == 0 {
            return (0, 0);
        }

        let control = self.io_read_u16_raw(REG_SOUNDCNT_H);
        let mut mono = 0i32;

        for (channel, sample) in [
            (0usize, self.direct_sound_a_sample as i32),
            (1usize, self.direct_sound_b_sample as i32),
        ] {
            let volume_shift = if control & (1 << (2 + channel)) != 0 { 2 } else { 1 };
            let contribution = sample << volume_shift;
            let right_enable_bit = 8 + channel * 4;
            let left_enable_bit = 9 + channel * 4;
            // The exported PCM is mono duplicated, so accumulate any routed
            // direct-sound channel once into the mono mixdown instead of
            // preserving hardware left/right separation here.
            if control & ((1 << right_enable_bit) | (1 << left_enable_bit)) != 0 {
                mono += contribution;
            }
        }

        let bias = (self.io_read_u16_raw(REG_SOUNDBIAS) & 0x03fe) as i32;
        let mono = ((mono + bias).clamp(0, 0x3ff) - bias) * AUDIO_OUTPUT_SCALE;
        (mono, mono)
    }

    fn fifo_for_offset(&mut self, offset: usize) -> Option<&mut VecDeque<i8>> {
        match offset {
            REG_FIFO_A..=0x0a3 => Some(&mut self.fifo_a),
            REG_FIFO_B..=0x0a7 => Some(&mut self.fifo_b),
            _ => None,
        }
    }

    fn push_fifo_byte(&mut self, offset: usize, value: u8) {
        if let Some(fifo) = self.fifo_for_offset(offset) {
            if fifo.len() < DIRECT_SOUND_FIFO_CAPACITY {
                let sample = value as i8;
                fifo.push_back(sample);
                let debug = if offset < REG_FIFO_B {
                    &mut self.debug_sound_a_writes
                } else {
                    &mut self.debug_sound_b_writes
                };
                Self::push_debug_sample(debug, sample);
                if self.active_dma_channel.is_none() {
                    let debug = if offset < REG_FIFO_B {
                        &mut self.debug_sound_a_cpu_writes
                    } else {
                        &mut self.debug_sound_b_cpu_writes
                    };
                    Self::push_debug_sample(debug, sample);
                }
            }
        }
    }

    fn push_fifo_halfword(&mut self, offset: usize, value: u16) {
        self.push_fifo_byte(offset, value as u8);
        self.push_fifo_byte(offset + 1, (value >> 8) as u8);
    }

    fn clear_sound_state(&mut self) {
        self.io[0x060..=0x081].fill(0);
        self.fifo_a.clear();
        self.fifo_b.clear();
        self.direct_sound_a_sample = 0;
        self.direct_sound_b_sample = 0;
    }

    fn prime_sound_dma_fifo(&mut self, fifo_addr: u32) {
        loop {
            let len = if fifo_addr == 0x0400_00a0 {
                self.fifo_a.len()
            } else {
                self.fifo_b.len()
            };
            if len > DIRECT_SOUND_FIFO_DMA_THRESHOLD {
                break;
            }
            self.run_dma_sound_request(fifo_addr);
            let new_len = if fifo_addr == 0x0400_00a0 {
                self.fifo_a.len()
            } else {
                self.fifo_b.len()
            };
            if new_len <= len {
                break;
            }
        }
    }

    fn clock_direct_sound_channel(&mut self, channel: usize) {
        let sample = {
            let fifo = if channel == 0 { &mut self.fifo_a } else { &mut self.fifo_b };
            let sample = fifo.pop_front().unwrap_or(0);
            if fifo.len() <= DIRECT_SOUND_FIFO_DMA_THRESHOLD {
                let fifo_addr = if channel == 0 { 0x0400_00a0 } else { 0x0400_00a4 };
                self.run_dma_sound_request(fifo_addr);
            }
            sample
        };

        if channel == 0 {
            self.direct_sound_a_sample = sample;
            Self::push_debug_sample(&mut self.debug_sound_a_pops, sample);
        } else {
            self.direct_sound_b_sample = sample;
            Self::push_debug_sample(&mut self.debug_sound_b_pops, sample);
        }
    }

    fn push_debug_sample(debug: &mut Vec<i8>, sample: i8) {
        if debug.len() == 64 {
            debug.remove(0);
        }
        debug.push(sample);
    }

    fn handle_timer_overflow(&mut self, timer: usize, overflows: u32) {
        if overflows == 0 || self.io[REG_SOUNDCNT_X] & 0x80 == 0 {
            return;
        }
        let soundcnt_h = self.io_read_u16_raw(REG_SOUNDCNT_H);
        let sound_a_timer = if soundcnt_h & (1 << 10) != 0 { 1 } else { 0 };
        let sound_b_timer = if soundcnt_h & (1 << 14) != 0 { 1 } else { 0 };

        for _ in 0..overflows {
            if timer == sound_a_timer {
                self.clock_direct_sound_channel(0);
            }
            if timer == sound_b_timer {
                self.clock_direct_sound_channel(1);
            }
        }
    }

    fn advance_time(&mut self, cycles: u32) {
        self.advance_time_internal(cycles, false);
    }

    fn advance_time_internal(&mut self, mut cycles: u32, stop_on_wakeup: bool) -> u32 {
        let original = cycles;
        while cycles > 0 {
            let line = self.frame_cycle / CYCLES_PER_LINE;
            let line_cycle = self.frame_cycle % CYCLES_PER_LINE;
            let to_hblank = if line_cycle < HDRAW_CYCLES {
                HDRAW_CYCLES - line_cycle
            } else {
                CYCLES_PER_LINE - line_cycle
            };
            let to_line_end = CYCLES_PER_LINE - line_cycle;
            let to_frame_end = FRAME_CYCLES - self.frame_cycle;
            let to_timer = self.cycles_to_next_timer_event();
            let to_audio = self.cycles_to_next_audio_pair();
            let step = cycles.min(to_hblank.min(to_line_end).min(to_frame_end).min(to_timer).min(to_audio));

            self.emit_audio_for_cycles(step);
            self.advance_timers(step);
            self.frame_cycle += step;
            cycles -= step;

            let new_line_cycle = self.frame_cycle % CYCLES_PER_LINE;
            if new_line_cycle == HDRAW_CYCLES {
                self.on_hblank_start(line);
            }

            if self.frame_cycle == FRAME_CYCLES {
                self.frame_cycle = 0;
                self.on_line_start(0);
            } else if new_line_cycle == 0 {
                let new_line = self.frame_cycle / CYCLES_PER_LINE;
                self.on_line_start(new_line);
            }

            if stop_on_wakeup && !self.halted && !self.stopped {
                break;
            }
        }
        original - cycles
    }

    fn on_line_start(&mut self, line: u32) {
        if line == VISIBLE_LINES {
            ppu::render_framebuffer(&self.io, &self.palette, &self.vram, &self.oam, &mut self.framebuffer);
            self.run_dma_timing(DmaTiming::VBlank);
            if self.io_read_u16_raw(REG_DISPSTAT) & (1 << 3) != 0 {
                self.raise_interrupt(IRQ_VBLANK);
            }
        }

        let dispstat = self.io_read_u16_raw(REG_DISPSTAT) & !0x0007;
        self.io_write_u16_raw(REG_DISPSTAT, dispstat);

        if line == self.vcount_target() as u32 && self.io_read_u16_raw(REG_DISPSTAT) & (1 << 5) != 0 {
            self.raise_interrupt(IRQ_VCOUNT);
        }

        self.poll_halt_wakeup();
    }

    fn on_hblank_start(&mut self, line: u32) {
        if line < VISIBLE_LINES {
            self.run_dma_timing(DmaTiming::HBlank);
            if self.io_read_u16_raw(REG_DISPSTAT) & (1 << 4) != 0 {
                self.raise_interrupt(IRQ_HBLANK);
            }
        }
        self.poll_halt_wakeup();
    }

    fn vcount(&self) -> u16 {
        (self.frame_cycle / CYCLES_PER_LINE) as u16
    }

    fn vcount_target(&self) -> u16 {
        (self.io_read_u16_raw(REG_DISPSTAT) >> 8) & 0xff
    }

    fn computed_dispstat(&self) -> u16 {
        let mut value = self.io_read_u16_raw(REG_DISPSTAT) & !0x0007;
        let line = self.vcount();
        let line_cycle = self.frame_cycle % CYCLES_PER_LINE;
        if line as u32 >= VISIBLE_LINES {
            value |= 1 << 0;
        }
        if line_cycle >= HDRAW_CYCLES {
            value |= 1 << 1;
        }
        if line == self.vcount_target() {
            value |= 1 << 2;
        }
        value
    }

    fn raise_interrupt(&mut self, irq_mask: u16) {
        let prev = self.io_read_u16_raw(REG_IF);
        self.io_write_u16_raw(REG_IF, prev | irq_mask);

        self.poll_halt_wakeup();
    }

    fn poll_halt_wakeup(&mut self) {
        let ie = self.io_read_u16_raw(REG_IE);
        let irq_flag_mem = self.read_u16_mapped(0x03ff_fff8);
        let pending = (self.io_read_u16_raw(REG_IF) | irq_flag_mem) & ie;
        if pending != 0 {
            self.halted = false;
            self.stopped = false;
        }
    }

    fn io_read_u8(&self, offset: usize) -> u8 {
        match offset {
            REG_DISPSTAT => (self.computed_dispstat() & 0xff) as u8,
            val if val == REG_DISPSTAT + 1 => (self.computed_dispstat() >> 8) as u8,
            REG_VCOUNT => (self.vcount() & 0xff) as u8,
            val if val == REG_VCOUNT + 1 => (self.vcount() >> 8) as u8,
            offset if (REG_TM0CNT_L..REG_TM0CNT_L + 0x10).contains(&offset) => {
                let timer = (offset - REG_TM0CNT_L) / 4;
                let timer_offset = (offset - REG_TM0CNT_L) % 4;
                match timer_offset {
                    0 => self.timer_counter[timer] as u8,
                    1 => (self.timer_counter[timer] >> 8) as u8,
                    _ => self.io[offset],
                }
            }
            REG_KEYINPUT => (!self.keys & 0xff) as u8,
            val if val == REG_KEYINPUT + 1 => 0xfc | (((!self.keys) >> 8) & 0x03) as u8,
            _ if offset < IO_SIZE => self.io[offset],
            _ => 0,
        }
    }

    fn io_write_u8_raw(&mut self, offset: usize, value: u8) {
        if offset < IO_SIZE {
            self.io[offset] = value;
        }
    }

    fn io_write_u16_raw(&mut self, offset: usize, value: u16) {
        self.io_write_u8_raw(offset, value as u8);
        self.io_write_u8_raw(offset + 1, (value >> 8) as u8);
    }

    #[cfg(test)]
    fn io_write_u32_raw(&mut self, offset: usize, value: u32) {
        self.io_write_u16_raw(offset, value as u16);
        self.io_write_u16_raw(offset + 2, (value >> 16) as u16);
    }

    fn write_io_u8(&mut self, offset: usize, value: u8) {
        if offset >= IO_SIZE {
            return;
        }
        if (REG_FIFO_A..=0x0a7).contains(&offset) {
            self.push_fifo_byte(offset, value);
            return;
        }
        let reg = offset & !1;
        let timer_control_write = TIMER_REG_BASES
            .iter()
            .position(|base| reg == *base + 2)
            .map(|timer| (timer, self.io_read_u16_raw(reg)));
        let dma_control_write = DMA_REG_BASES
            .iter()
            .position(|base| reg == *base + 0x0a)
            .map(|channel| (channel, self.io_read_u16_raw(reg)));
        let old = if reg == REG_IF {
            self.io_read_u16_raw(REG_IF)
        } else {
            0
        };
        self.io[offset] = value;
        if reg == REG_DISPCNT {
            self.debug_dispcnt_writes = self.debug_dispcnt_writes.wrapping_add(1);
            self.debug_last_dispcnt_write_pc = self.cpu.pc();
            self.debug_last_dispcnt_value = self.io_read_u16_raw(REG_DISPCNT);
        }
        if reg == REG_IE {
            self.debug_last_ie_write_pc = self.cpu.pc();
            self.debug_last_ie_value = self.io_read_u16_raw(REG_IE);
        }
        if reg + 1 == REG_HALTCNT {
            if offset == REG_HALTCNT {
                self.debug_last_haltcnt_write_pc = self.cpu.pc();
                self.debug_last_haltcnt_value = self.io[REG_HALTCNT];
                self.handle_haltcnt_write();
            }
            return;
        }
        if let Some((timer, old_control)) = timer_control_write {
            self.handle_timer_control_write(timer, old_control);
            return;
        }
        if let Some((channel, old_control)) = dma_control_write {
            self.handle_dma_control_write(channel, old_control);
            return;
        }
        self.handle_io_write(reg, old);
    }

    fn write_io_u16(&mut self, offset: usize, value: u16) {
        if offset + 1 >= IO_SIZE {
            return;
        }
        if (REG_FIFO_A..=0x0a6).contains(&offset) {
            self.push_fifo_halfword(offset, value);
            return;
        }
        let timer_control_write = TIMER_REG_BASES
            .iter()
            .position(|base| offset == *base + 2)
            .map(|timer| (timer, self.io_read_u16_raw(offset)));
        let dma_control_write = DMA_REG_BASES
            .iter()
            .position(|base| offset == *base + 0x0a)
            .map(|channel| (channel, self.io_read_u16_raw(offset)));
        let old = if offset == REG_IF {
            self.io_read_u16_raw(REG_IF)
        } else {
            0
        };
        self.io[offset] = value as u8;
        self.io[offset + 1] = (value >> 8) as u8;
        if offset == REG_DISPCNT {
            self.debug_dispcnt_writes = self.debug_dispcnt_writes.wrapping_add(1);
            self.debug_last_dispcnt_write_pc = self.cpu.pc();
            self.debug_last_dispcnt_value = value;
        }
        if offset == REG_IE {
            self.debug_last_ie_write_pc = self.cpu.pc();
            self.debug_last_ie_value = value;
        }
        if offset + 1 == REG_HALTCNT {
            self.debug_last_haltcnt_write_pc = self.cpu.pc();
            self.debug_last_haltcnt_value = self.io[REG_HALTCNT];
        }
        if let Some((timer, old_control)) = timer_control_write {
            self.handle_timer_control_write(timer, old_control);
            return;
        }
        if let Some((channel, old_control)) = dma_control_write {
            self.handle_dma_control_write(channel, old_control);
            return;
        }
        self.handle_io_write(offset, old);
    }

    fn write_io_u32(&mut self, offset: usize, value: u32) {
        self.write_io_u16(offset, value as u16);
        self.write_io_u16(offset + 2, (value >> 16) as u16);
    }

    fn handle_io_write(&mut self, offset: usize, old_if: u16) {
        match offset {
            REG_DISPSTAT => {
                let write = self.io_read_u16_raw(REG_DISPSTAT);
                let preserved = self.computed_dispstat() & 0x0007;
                self.io_write_u16_raw(REG_DISPSTAT, (write & !0x0007) | preserved);
            }
            REG_VCOUNT => {}
            REG_KEYINPUT => {}
            REG_IF => {
                let write_mask = self.io_read_u16_raw(REG_IF);
                self.io_write_u16_raw(REG_IF, old_if & !write_mask);
            }
            REG_IME => {
                let ime = self.io[REG_IME] & 1;
                self.io[REG_IME] = ime;
                self.poll_halt_wakeup();
            }
            REG_SOUNDCNT_H => {
                let control = self.io_read_u16_raw(REG_SOUNDCNT_H);
                if control & (1 << 11) != 0 {
                    self.fifo_a.clear();
                    self.direct_sound_a_sample = 0;
                    self.prime_sound_dma_fifo(0x0400_00a0);
                }
                if control & (1 << 15) != 0 {
                    self.fifo_b.clear();
                    self.direct_sound_b_sample = 0;
                    self.prime_sound_dma_fifo(0x0400_00a4);
                }
            }
            REG_SOUNDCNT_X => {
                let enabled = self.io[REG_SOUNDCNT_X] & 0x80;
                self.io[REG_SOUNDCNT_X] = enabled;
                self.io[REG_SOUNDCNT_X + 1] = 0;
                if enabled == 0 {
                    self.clear_sound_state();
                }
            }
            _ if offset == REG_HALTCNT || offset + 1 == REG_HALTCNT => self.handle_haltcnt_write(),
            _ => {}
        }
    }

    fn handle_haltcnt_write(&mut self) {
        let value = self.io[REG_HALTCNT];
        if value & 0x80 == 0 {
            self.halted = true;
        } else {
            self.stopped = true;
        }
    }

    fn handle_timer_control_write(&mut self, timer: usize, old_control: u16) {
        let new_control = self.timer_control(timer);
        let old_enabled = old_control & (1 << 7) != 0;
        let new_enabled = new_control & (1 << 7) != 0;
        if !old_enabled && new_enabled {
            self.timer_counter[timer] = self.timer_reload(timer);
            self.timer_remainder[timer] = 0;
        } else if old_enabled && !new_enabled {
            self.timer_remainder[timer] = 0;
        }
    }

    fn reload_dma_channel(&mut self, channel: usize) {
        let base = DMA_REG_BASES[channel];
        self.dma_src[channel] = self.io_read_u32_raw(base);
        self.dma_dst[channel] = self.io_read_u32_raw(base + 0x04);
    }

    fn timer_reload(&self, timer: usize) -> u16 {
        self.io_read_u16_raw(TIMER_REG_BASES[timer])
    }

    fn timer_control(&self, timer: usize) -> u16 {
        self.io_read_u16_raw(TIMER_REG_BASES[timer] + 2)
    }

    fn timer_enabled(&self, timer: usize) -> bool {
        self.timer_control(timer) & (1 << 7) != 0
    }

    fn timer_count_up(&self, timer: usize) -> bool {
        timer != 0 && self.timer_control(timer) & (1 << 2) != 0
    }

    fn timer_irq_enabled(&self, timer: usize) -> bool {
        self.timer_control(timer) & (1 << 6) != 0
    }

    fn timer_prescaler(&self, timer: usize) -> u32 {
        TIMER_PRESCALERS[(self.timer_control(timer) & 0x3) as usize]
    }

    fn cycles_to_next_timer_event(&self) -> u32 {
        let mut next = u32::MAX;
        for timer in 0..4 {
            if !self.timer_enabled(timer) || self.timer_count_up(timer) {
                continue;
            }
            let ticks_until_overflow = 0x1_0000 - self.timer_counter[timer] as u32;
            let cycles_until_overflow =
                ticks_until_overflow.saturating_mul(self.timer_prescaler(timer)) - self.timer_remainder[timer];
            next = next.min(cycles_until_overflow.max(1));
        }
        next
    }

    fn advance_timers(&mut self, cycles: u32) {
        if cycles == 0 {
            return;
        }

        let mut overflows = [0u32; 4];
        for timer in 0..4 {
            if !self.timer_enabled(timer) || self.timer_count_up(timer) {
                continue;
            }

            let prescaler = self.timer_prescaler(timer);
            let total_cycles = self.timer_remainder[timer] + cycles;
            let increments = total_cycles / prescaler;
            self.timer_remainder[timer] = total_cycles % prescaler;
            overflows[timer] = self.advance_timer_counter(timer, increments);
        }

        for timer in 1..4 {
            if !self.timer_enabled(timer) || !self.timer_count_up(timer) {
                continue;
            }
            overflows[timer] = self.advance_timer_counter(timer, overflows[timer - 1]);
        }

        for (timer, count) in overflows.into_iter().enumerate() {
            self.handle_timer_overflow(timer, count);
        }
    }

    fn advance_timer_counter(&mut self, timer: usize, increments: u32) -> u32 {
        if increments == 0 {
            return 0;
        }

        let current = self.timer_counter[timer] as u32;
        let reload = self.timer_reload(timer) as u32;
        let ticks_until_overflow = 0x1_0000 - current;
        if increments < ticks_until_overflow {
            self.timer_counter[timer] = current.wrapping_add(increments) as u16;
            return 0;
        }

        let period = 0x1_0000 - reload;
        let remaining = increments - ticks_until_overflow;
        let extra_overflows = remaining / period;
        let final_ticks = remaining % period;
        let overflows = 1 + extra_overflows;
        self.timer_counter[timer] = reload.wrapping_add(final_ticks) as u16;

        if self.timer_irq_enabled(timer) {
            self.raise_interrupt(TIMER_IRQS[timer]);
        }

        overflows
    }

    fn handle_dma_control_write(&mut self, channel: usize, old_control: u16) {
        let base = DMA_REG_BASES[channel];
        let control = self.io_read_u16_raw(base + 0x0a);
        let old_enabled = old_control & (1 << 15) != 0;
        let new_enabled = control & (1 << 15) != 0;
        if !old_enabled && new_enabled {
            self.reload_dma_channel(channel);
        }
        if !new_enabled {
            return;
        }
        if self.dma_timing(channel) == DmaTiming::Immediate {
            self.run_dma_channel(channel);
            return;
        }
        if !old_enabled && self.dma_timing(channel) == DmaTiming::Special && (1..=2).contains(&channel) {
            let fifo_addr = self.io_read_u32_raw(base + 0x04);
            if fifo_addr == 0x0400_00a0 || fifo_addr == 0x0400_00a4 {
                self.prime_sound_dma_fifo(fifo_addr);
            }
        }
    }

    fn dma_timing(&self, channel: usize) -> DmaTiming {
        let base = DMA_REG_BASES[channel];
        match (self.io_read_u16_raw(base + 0x0a) >> 12) & 0x3 {
            0 => DmaTiming::Immediate,
            1 => DmaTiming::VBlank,
            2 => DmaTiming::HBlank,
            _ => DmaTiming::Special,
        }
    }

    fn run_dma_timing(&mut self, timing: DmaTiming) {
        for channel in 0..4 {
            let base = DMA_REG_BASES[channel];
            let control = self.io_read_u16_raw(base + 0x0a);
            if control & (1 << 15) == 0 || self.dma_timing(channel) != timing {
                continue;
            }
            self.run_dma_channel(channel);
        }
    }

    fn run_dma_channel(&mut self, channel: usize) {
        self.run_dma_channel_inner(channel, false);
    }

    fn run_dma_sound_request(&mut self, fifo_addr: u32) {
        for channel in 1..=2 {
            let base = DMA_REG_BASES[channel];
            let control = self.io_read_u16_raw(base + 0x0a);
            if control & (1 << 15) == 0 || self.dma_timing(channel) != DmaTiming::Special {
                continue;
            }
            if self.io_read_u32_raw(base + 0x04) != fifo_addr {
                continue;
            }
            self.run_dma_channel_inner(channel, true);
        }
    }

    fn run_dma_channel_inner(&mut self, channel: usize, sound_fifo: bool) {
        let prev_dma_channel = self.active_dma_channel.replace(channel);
        let base = DMA_REG_BASES[channel];
        let control = self.io_read_u16_raw(base + 0x0a);
        let mut src = self.dma_src[channel];
        let mut dst = self.dma_dst[channel];
        let mut count = self.io_read_u16_raw(base + 0x08) as u32;
        let word = sound_fifo || control & (1 << 10) != 0;
        let repeat = control & (1 << 9) != 0 && self.dma_timing(channel) != DmaTiming::Immediate;
        let dst_mode = (control >> 5) & 0x3;
        let src_mode = (control >> 7) & 0x3;

        if sound_fifo {
            count = 4;
        } else if count == 0 {
            count = if channel == 3 { 0x1_0000 } else { 0x4_000 };
        }

        let transfer_size = if word { 4 } else { 2 };
        for _ in 0..count {
            if word {
                let value = self.read_u32_mapped(src);
                self.write_u32_mapped(dst, value);
            } else {
                let value = self.read_u16_mapped(src);
                self.write_u16_mapped(dst, value);
            }

            src = match src_mode {
                0 | 3 => src.wrapping_add(transfer_size),
                1 => src.wrapping_sub(transfer_size),
                2 => src,
                _ => src,
            };

            dst = match dst_mode {
                _ if sound_fifo => dst,
                0 | 3 => dst.wrapping_add(transfer_size),
                1 => dst.wrapping_sub(transfer_size),
                2 => dst,
                _ => dst,
            };
        }

        if control & (1 << 14) != 0 {
            self.raise_interrupt(1 << (8 + channel));
        }

        self.dma_src[channel] = src;
        self.dma_dst[channel] = if repeat && !sound_fifo && dst_mode == 3 {
            self.io_read_u32_raw(base + 0x04)
        } else {
            dst
        };

        if !repeat {
            self.io_write_u16_raw(base + 0x0a, control & !(1 << 15));
        }
        self.active_dma_channel = prev_dma_channel;
    }

    fn io_read_u16_raw(&self, offset: usize) -> u16 {
        let lo = self.io[offset] as u16;
        let hi = self.io[offset + 1] as u16;
        lo | (hi << 8)
    }

    fn io_read_u32_raw(&self, offset: usize) -> u32 {
        let lo = self.io_read_u16_raw(offset) as u32;
        let hi = self.io_read_u16_raw(offset + 2) as u32;
        lo | (hi << 16)
    }

    fn map_vram_addr(addr: u32) -> usize {
        let mut offset = (addr as usize).wrapping_sub(0x0600_0000) & 0x1ffff;
        if offset >= 0x18000 {
            offset -= 0x8000;
        }
        offset % VRAM_SIZE
    }

    fn map_io_addr(addr: u32) -> Option<usize> {
        let offset = (addr as usize).wrapping_sub(0x0400_0000);
        if offset < IO_SIZE {
            Some(offset)
        } else {
            None
        }
    }

    fn read_u8_mapped(&self, addr: u32) -> u8 {
        match addr >> 24 {
            0x00 => {
                let offset = addr as usize & 0x3fff;
                self.bios[offset]
            }
            0x02 => {
                let offset = addr as usize & (EWRAM_SIZE - 1);
                self.ewram[offset]
            }
            0x03 => {
                let offset = addr as usize & (IWRAM_SIZE - 1);
                self.iwram[offset]
            }
            0x04 => {
                Self::map_io_addr(addr).map_or(0, |offset| self.io_read_u8(offset))
            }
            0x05 => {
                let offset = addr as usize & (PALETTE_SIZE - 1);
                self.palette[offset]
            }
            0x06 => {
                let offset = Self::map_vram_addr(addr);
                self.vram[offset]
            }
            0x07 => {
                let offset = addr as usize & (OAM_SIZE - 1);
                self.oam[offset]
            }
            0x08..=0x0d => {
                let offset = (addr as usize).wrapping_sub(0x0800_0000) & (ROM_BUFFER_SIZE - 1);
                if offset < self.rom_len {
                    self.rom_staging[offset]
                } else {
                    0
                }
            }
            0x0e => {
                let offset = addr as usize & (SRAM_SIZE - 1);
                self.sram[offset]
            }
            _ => 0,
        }
    }

    fn read_u16_mapped(&self, addr: u32) -> u16 {
        let lo = self.read_u8_mapped(addr) as u16;
        let hi = self.read_u8_mapped(addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    fn read_u16_aligned(&self, addr: u32) -> u16 {
        let aligned = addr & !1;
        let value = self.read_u16_mapped(aligned);
        if addr & 1 == 0 {
            value
        } else {
            value.rotate_right(8)
        }
    }

    fn read_u32_mapped(&self, addr: u32) -> u32 {
        let aligned = addr & !3;
        let b0 = self.read_u8_mapped(aligned) as u32;
        let b1 = self.read_u8_mapped(aligned.wrapping_add(1)) as u32;
        let b2 = self.read_u8_mapped(aligned.wrapping_add(2)) as u32;
        let b3 = self.read_u8_mapped(aligned.wrapping_add(3)) as u32;
        let value = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
        value.rotate_right((addr & 3) * 8)
    }

    fn write_u8_mapped(&mut self, addr: u32, value: u8) {
        match addr >> 24 {
            0x02 => {
                let offset = addr as usize & (EWRAM_SIZE - 1);
                self.ewram[offset] = value;
            }
            0x03 => {
                let offset = addr as usize & (IWRAM_SIZE - 1);
                self.iwram[offset] = value;
            }
            0x04 => {
                if let Some(offset) = Self::map_io_addr(addr) {
                    self.write_io_u8(offset, value);
                }
            }
            0x05 => {
                let offset = addr as usize & (PALETTE_SIZE - 1);
                self.palette[offset] = value;
            }
            0x06 => {
                let offset = Self::map_vram_addr(addr);
                self.vram[offset] = value;
            }
            0x07 => {
                let offset = addr as usize & (OAM_SIZE - 1);
                self.oam[offset] = value;
            }
            0x0e => {
                let offset = addr as usize & (SRAM_SIZE - 1);
                self.sram[offset] = value;
            }
            _ => {}
        }
    }

    fn write_u16_mapped(&mut self, addr: u32, value: u16) {
        match addr >> 24 {
            0x02 => {
                let aligned = (addr as usize & !1) & (EWRAM_SIZE - 1);
                self.ewram[aligned] = value as u8;
                self.ewram[(aligned + 1) & (EWRAM_SIZE - 1)] = (value >> 8) as u8;
            }
            0x03 => {
                let aligned = (addr as usize & !1) & (IWRAM_SIZE - 1);
                self.iwram[aligned] = value as u8;
                self.iwram[(aligned + 1) & (IWRAM_SIZE - 1)] = (value >> 8) as u8;
            }
            0x04 => {
                if let Some(offset) = Self::map_io_addr(addr & !1) {
                    self.write_io_u16(offset, value);
                }
            }
            0x05 => {
                let aligned = (addr as usize & !1) & (PALETTE_SIZE - 1);
                self.palette[aligned] = value as u8;
                self.palette[(aligned + 1) & (PALETTE_SIZE - 1)] = (value >> 8) as u8;
            }
            0x06 => {
                let aligned = Self::map_vram_addr(addr & !1);
                self.vram[aligned] = value as u8;
                let next = Self::map_vram_addr((addr & !1).wrapping_add(1));
                self.vram[next] = (value >> 8) as u8;
            }
            0x07 => {
                let aligned = (addr as usize & !1) & (OAM_SIZE - 1);
                self.oam[aligned] = value as u8;
                self.oam[(aligned + 1) & (OAM_SIZE - 1)] = (value >> 8) as u8;
            }
            0x0e => {
                self.write_u8_mapped(addr, value as u8);
                self.write_u8_mapped(addr.wrapping_add(1), (value >> 8) as u8);
            }
            _ => {}
        }
    }

    fn write_u32_mapped(&mut self, addr: u32, value: u32) {
        match addr >> 24 {
            0x02 => {
                let base = (addr as usize & !3) & (EWRAM_SIZE - 1);
                self.ewram[base] = value as u8;
                self.ewram[(base + 1) & (EWRAM_SIZE - 1)] = (value >> 8) as u8;
                self.ewram[(base + 2) & (EWRAM_SIZE - 1)] = (value >> 16) as u8;
                self.ewram[(base + 3) & (EWRAM_SIZE - 1)] = (value >> 24) as u8;
            }
            0x03 => {
                let base = (addr as usize & !3) & (IWRAM_SIZE - 1);
                self.iwram[base] = value as u8;
                self.iwram[(base + 1) & (IWRAM_SIZE - 1)] = (value >> 8) as u8;
                self.iwram[(base + 2) & (IWRAM_SIZE - 1)] = (value >> 16) as u8;
                self.iwram[(base + 3) & (IWRAM_SIZE - 1)] = (value >> 24) as u8;
            }
            0x04 => {
                if let Some(offset) = Self::map_io_addr(addr & !3) {
                    self.write_io_u32(offset, value);
                }
            }
            0x05 => {
                let base = (addr as usize & !3) & (PALETTE_SIZE - 1);
                self.palette[base] = value as u8;
                self.palette[(base + 1) & (PALETTE_SIZE - 1)] = (value >> 8) as u8;
                self.palette[(base + 2) & (PALETTE_SIZE - 1)] = (value >> 16) as u8;
                self.palette[(base + 3) & (PALETTE_SIZE - 1)] = (value >> 24) as u8;
            }
            0x06 => {
                let base_addr = addr & !3;
                for i in 0..4 {
                    let offset = Self::map_vram_addr(base_addr.wrapping_add(i));
                    self.vram[offset] = (value >> (i * 8)) as u8;
                }
            }
            0x07 => {
                let base = (addr as usize & !3) & (OAM_SIZE - 1);
                self.oam[base] = value as u8;
                self.oam[(base + 1) & (OAM_SIZE - 1)] = (value >> 8) as u8;
                self.oam[(base + 2) & (OAM_SIZE - 1)] = (value >> 16) as u8;
                self.oam[(base + 3) & (OAM_SIZE - 1)] = (value >> 24) as u8;
            }
            0x0e => {
                self.write_u8_mapped(addr, value as u8);
                self.write_u8_mapped(addr.wrapping_add(1), (value >> 8) as u8);
                self.write_u8_mapped(addr.wrapping_add(2), (value >> 16) as u8);
                self.write_u8_mapped(addr.wrapping_add(3), (value >> 24) as u8);
            }
            _ => {}
        }
    }
}

fn global_emu() -> &'static mut Emulator {
    let cell = EMULATOR.get_or_init(|| GlobalEmulator(UnsafeCell::new(Emulator::new())));
    unsafe { &mut *cell.0.get() }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct NativeEmulator {
    inner: Emulator,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeEmulator {
    pub fn new_with_rom(rom: &[u8]) -> Option<Self> {
        let mut inner = Emulator::new();
        if rom.len() > inner.rom_staging.len() {
            return None;
        }
        inner.rom_staging[..rom.len()].copy_from_slice(rom);
        if !inner.load_rom(rom.len()) {
            return None;
        }
        Some(Self { inner })
    }

    pub fn reset(&mut self) {
        self.inner.reset_runtime_state();
    }

    pub fn set_audio_delay_pairs_for_debug(&mut self, delay_pairs: usize) {
        self.inner.audio_delay_pairs = delay_pairs.max(1);
        self.inner.reset_audio_output_history_for_debug();
    }

    pub fn set_audio_first_pair_cycles_for_debug(&mut self, cycles: u32) {
        let rate = self.inner.audio_rate().max(1) as u64;
        let period_cycles = (CPU_CLOCK_HZ as u64 / rate).max(1) as u32;
        let clamped_cycles = cycles.clamp(1, period_cycles);
        let initial_fraction = CPU_CLOCK_HZ as u64 - rate * clamped_cycles as u64;
        self.inner.initial_audio_fraction = initial_fraction;
        self.inner.audio_fraction = initial_fraction;
        self.inner.reset_audio_output_history_for_debug();
    }

    pub fn set_audio_capture_mode_for_debug(&mut self, mode: &str) -> Result<(), String> {
        self.inner.audio_capture_mode = match mode {
            "average" => AudioCaptureMode::Average,
            "endpoint" => AudioCaptureMode::Endpoint,
            _ => {
                return Err(format!(
                    "invalid audio capture mode: {mode} (expected average or endpoint)"
                ))
            }
        };
        self.inner.reset_audio_output_history_for_debug();
        Ok(())
    }

    pub fn set_audio_prefilter_gain_num_for_debug(&mut self, gain_num: i32) {
        self.inner.audio_output_params.prefilter_gain_num = gain_num.clamp(120, 144);
        self.inner.reset_audio_output_history_for_debug();
    }

    pub fn set_audio_params_for_debug(&mut self, spec: &str) -> Result<(), String> {
        self.inner.set_audio_params_for_debug(spec)
    }

    pub fn audio_params_for_debug(&self) -> String {
        self.inner.audio_params_for_debug()
    }

    pub fn set_keys(&mut self, keys: u32) {
        self.inner.keys = (keys & 0x03ff) as u16;
    }

    pub fn run_frame(&mut self) {
        self.inner.run_frame();
    }

    pub fn step_instruction(&mut self) -> u32 {
        if self.inner.rom_len == 0 || self.inner.stopped {
            return 0;
        }
        if self.inner.halted && !self.inner.irq_pending() {
            return 0;
        }
        let used = if self.inner.irq_pending() {
            self.inner.service_irq()
        } else {
            self.inner.step_cpu().max(1)
        };
        self.inner.advance_time(used);
        used
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.inner.framebuffer
    }

    pub fn take_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inner.audio_buffer)
    }

    pub fn take_pair_input_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inner.audio_pair_input_buffer)
    }

    pub fn take_prefilter_input_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inner.audio_prefilter_input_buffer)
    }

    pub fn take_prefilter_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inner.audio_prefilter_buffer)
    }

    pub fn audio_rate(&self) -> i32 {
        self.inner.audio_rate()
    }

    pub fn pc(&self) -> u32 {
        self.inner.cpu.pc()
    }

    pub fn cpsr(&self) -> u32 {
        self.inner.cpu.cpsr()
    }

    pub fn thumb(&self) -> bool {
        self.inner.cpu.thumb()
    }

    pub fn halted(&self) -> bool {
        self.inner.halted
    }

    pub fn stopped(&self) -> bool {
        self.inner.stopped
    }

    pub fn dispcnt(&self) -> u16 {
        self.inner.io_read_u16_raw(REG_DISPCNT)
    }

    pub fn vcount(&self) -> u16 {
        self.inner.vcount()
    }

    pub fn frame_cycle(&self) -> u32 {
        self.inner.frame_cycle
    }

    pub fn frames_emulated(&self) -> u64 {
        self.inner.frames_emulated
    }

    pub fn registers(&self) -> [u32; 16] {
        self.inner.cpu.registers()
    }

    pub fn instruction_count(&self) -> u64 {
        self.inner.debug_instruction_count
    }

    pub fn dispcnt_writes(&self) -> u32 {
        self.inner.debug_dispcnt_writes
    }

    pub fn last_dispcnt_write_pc(&self) -> u32 {
        self.inner.debug_last_dispcnt_write_pc
    }

    pub fn last_dispcnt_value(&self) -> u16 {
        self.inner.debug_last_dispcnt_value
    }

    pub fn last_ie_write_pc(&self) -> u32 {
        self.inner.debug_last_ie_write_pc
    }

    pub fn last_ie_value(&self) -> u16 {
        self.inner.debug_last_ie_value
    }

    pub fn last_haltcnt_write_pc(&self) -> u32 {
        self.inner.debug_last_haltcnt_write_pc
    }

    pub fn last_haltcnt_value(&self) -> u8 {
        self.inner.debug_last_haltcnt_value
    }

    pub fn peek_u32(&self, addr: u32) -> u32 {
        self.inner.read_u32_mapped(addr)
    }

    pub fn direct_sound_a_sample(&self) -> i8 {
        self.inner.direct_sound_a_sample
    }

    pub fn direct_sound_b_sample(&self) -> i8 {
        self.inner.direct_sound_b_sample
    }

    pub fn fifo_a_len(&self) -> usize {
        self.inner.fifo_a.len()
    }

    pub fn fifo_b_len(&self) -> usize {
        self.inner.fifo_b.len()
    }

    pub fn debug_sound_a_writes(&self) -> &[i8] {
        &self.inner.debug_sound_a_writes
    }

    pub fn debug_sound_b_writes(&self) -> &[i8] {
        &self.inner.debug_sound_b_writes
    }

    pub fn debug_sound_a_pops(&self) -> &[i8] {
        &self.inner.debug_sound_a_pops
    }

    pub fn debug_sound_b_pops(&self) -> &[i8] {
        &self.inner.debug_sound_b_pops
    }

    pub fn debug_sound_a_cpu_writes(&self) -> &[i8] {
        &self.inner.debug_sound_a_cpu_writes
    }

    pub fn debug_sound_b_cpu_writes(&self) -> &[i8] {
        &self.inner.debug_sound_b_cpu_writes
    }
}

#[no_mangle]
pub extern "C" fn emu_init() -> i32 {
    let _ = global_emu();
    1
}

#[no_mangle]
pub extern "C" fn emu_rom_buffer() -> *mut u8 {
    global_emu().rom_staging.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn emu_load_rom(len: i32) -> i32 {
    if len < 0 {
        return 0;
    }
    global_emu().load_rom(len as usize) as i32
}

#[no_mangle]
pub extern "C" fn emu_reset() -> i32 {
    let emu = global_emu();
    if emu.rom_len == 0 {
        return 0;
    }
    emu.reset_runtime_state();
    1
}

#[no_mangle]
pub extern "C" fn emu_set_keys(keys: u32) {
    global_emu().keys = (keys & 0x03ff) as u16;
}

#[no_mangle]
pub extern "C" fn emu_run_frame() {
    let emu = global_emu();
    if emu.rom_len != 0 {
        emu.run_frame();
    }
}

#[no_mangle]
pub extern "C" fn emu_framebuffer() -> *mut u32 {
    global_emu().framebuffer.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn emu_audio_buffer() -> *mut i16 {
    global_emu().audio_buffer.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn emu_audio_samples() -> i32 {
    let emu = global_emu();
    let pairs = (emu.audio_buffer.len() / 2) as i32;
    emu.audio_buffer.clear();
    pairs
}

#[no_mangle]
pub extern "C" fn emu_audio_rate() -> i32 {
    global_emu().audio_rate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_enters_bios() {
        let emu = Emulator::new();
        assert_eq!(emu.cpu.pc(), 0);
        assert_eq!(emu.cpu.mode(), Mode::Supervisor);
        assert!(!emu.cpu.thumb());
    }

    #[test]
    fn rom_buffer_is_large_enough() {
        let emu = Emulator::new();
        assert!(emu.rom_staging.len() >= ROM_BUFFER_SIZE);
    }

    #[test]
    fn smoke_boots_anguna_one_frame() {
        let mut emu = Emulator::new();
        let rom = std::fs::read("dev-roms/anguna.gba").expect("rom should be readable");
        emu.rom_staging[..rom.len()].copy_from_slice(&rom);
        assert!(emu.load_rom(rom.len()));
        emu.run_frame();
        assert_ne!(emu.cpu.pc(), 0);
        assert_eq!(emu.framebuffer.len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    }

    #[test]
    fn smoke_boots_xniq_one_frame() {
        let mut emu = Emulator::new();
        let rom = std::fs::read("dev-roms/xniq.gba").expect("rom should be readable");
        emu.rom_staging[..rom.len()].copy_from_slice(&rom);
        assert!(emu.load_rom(rom.len()));
        emu.run_frame();
        assert_ne!(emu.cpu.pc(), 0);
        assert_eq!(emu.framebuffer.len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    }

    #[test]
    fn halted_cpu_resumes_execution_after_hblank_wakeup() {
        let mut emu = Emulator::new();
        emu.cpu
            .debug_set_state((Mode::System as u32) | (1 << 5), 0x0300_0000);
        emu.iwram[0x0000..0x0004].copy_from_slice(&[0x01, 0x20, 0x02, 0x21]); // movs r0,#1; movs r1,#2
        emu.io_write_u16_raw(REG_IE, IRQ_HBLANK);
        emu.io_write_u16_raw(REG_DISPSTAT, 1 << 4);
        emu.halted = true;
        emu.frame_cycle = HDRAW_CYCLES - 1;

        emu.run_cycles(3);

        assert!(!emu.halted);
        assert_eq!(emu.cpu.debug_reg(0), 1);
        assert_eq!(emu.cpu.debug_reg(1), 2);
        assert_eq!(emu.cpu.pc(), 0x0300_0004);
        assert_eq!(emu.io_read_u16_raw(REG_IF) & IRQ_HBLANK, IRQ_HBLANK);
    }

    #[test]
    fn startup_audio_drain_matches_oracle_pair_schedule() {
        let mut emu = Emulator::new();
        let expected_pairs = [2049, 548, 549, 548, 549, 549, 548, 549, 549, 548, 549, 548];

        for (frame, expected) in expected_pairs.into_iter().enumerate() {
            emu.append_boot_audio_preroll();
            emu.emit_audio_for_cycles(FRAME_CYCLES);
            assert_eq!(
                emu.audio_buffer.len() / 2,
                expected,
                "unexpected audio pair count at frame {}",
                frame + 1
            );
            emu.audio_buffer.clear();
            emu.frames_emulated += 1;
        }
    }

    #[test]
    fn sound_fifo_dma_refills_on_timer_overflow() {
        let mut emu = Emulator::new();

        emu.write_io_u16(REG_SOUNDCNT_H, 0x030c);
        emu.write_io_u16(REG_SOUNDCNT_X, 0x0080);
        emu.write_io_u8(REG_FIFO_A, 0x7f);

        emu.ewram[0..16].copy_from_slice(&[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        ]);
        emu.io_write_u32_raw(0x0bc, 0x0200_0000);
        emu.io_write_u32_raw(0x0c0, 0x0400_00a0);
        emu.write_io_u16(0x0c6, 0xb600);
        emu.write_io_u16(REG_TM0CNT_L, 0xffff);
        emu.write_io_u16(REG_TM0CNT_H, 0x0080);

        emu.advance_time(1);

        assert_eq!(emu.direct_sound_a_sample, 0x7f);
        assert_eq!(emu.fifo_a.len(), DIRECT_SOUND_FIFO_CAPACITY);
        assert_eq!(emu.fifo_a.front().copied(), Some(1));
        assert_eq!(emu.io_read_u32_raw(0x0bc), 0x0200_0000);
        assert_eq!(emu.dma_src[1], 0x0200_0020);
    }

    #[test]
    fn direct_sound_mono_export_sums_split_channels() {
        let mut emu = Emulator::new();

        emu.write_io_u16(REG_SOUNDCNT_X, 0x0080);
        emu.write_io_u16(REG_SOUNDCNT_H, 0x120c);
        emu.direct_sound_a_sample = 0x10;
        emu.direct_sound_b_sample = 0x10;

        assert_eq!(emu.mix_audio_level(), (8_192, 8_192));
    }

    #[test]
    fn direct_sound_generates_nonzero_pcm() {
        let mut emu = Emulator::new();
        let outputs = AUDIO_OUTPUT_DELAY_PAIRS + AUDIO_OUTPUT_FILTER_TAPS.len() + 1;

        emu.write_io_u16(REG_SOUNDCNT_H, 0x0304);
        emu.write_io_u16(REG_SOUNDCNT_X, 0x0080);
        emu.write_io_u8(REG_FIFO_A, 0x10);
        emu.direct_sound_a_sample = 0x10;

        for _ in 0..outputs {
            emu.emit_audio_for_cycles(512);
        }

        assert_eq!(emu.audio_buffer.len(), outputs * 2);
        assert_eq!(emu.audio_prefilter_input_buffer.len(), outputs * 2);
        assert_eq!(emu.audio_prefilter_buffer.len(), outputs * 2);
        assert_eq!(
            &emu.audio_prefilter_input_buffer[emu.audio_prefilter_input_buffer.len() - 4..],
            &[1024, 1024, 1024, 1024]
        );
        assert_eq!(
            &emu.audio_prefilter_buffer[emu.audio_prefilter_buffer.len() - 4..],
            &[1020, 1020, 1020, 1020]
        );
        assert_eq!(
            &emu.audio_buffer[emu.audio_buffer.len() - 18..],
            &[995, 995, 1036, 1036, 1024, 1024, 1315, 1315, 967, 967, 1104, 1104, 1177, 1177, 1070, 1070, 1093, 1093]
        );
    }

    #[test]
    fn audio_output_compresses_large_peaks() {
        let mut emu = Emulator::new();
        emu.audio_delay_line
            .extend(std::iter::repeat_n(16_000, AUDIO_OUTPUT_DELAY_PAIRS));

        let (_, prefilter, output) = emu.filter_audio_output(0);

        assert_eq!(prefilter, 4_516);
        assert_eq!(output, 3_666);
    }

    #[test]
    fn audio_output_biases_negative_peaks_asymmetrically() {
        let mut emu = Emulator::new();
        emu.audio_delay_line
            .extend(std::iter::repeat_n(-16_000, AUDIO_OUTPUT_DELAY_PAIRS));

        let (_, prefilter, output) = emu.filter_audio_output(0);

        assert_eq!(prefilter, -4_516);
        assert_eq!(output, -3_502);
    }

    #[test]
    fn audio_debug_params_accept_tune_audio_output_lines() {
        let mut emu = Emulator::new();
        emu.audio_buffer.extend_from_slice(&[1, 2]);
        emu.audio_pair_input_buffer.extend_from_slice(&[3, 4]);
        emu.audio_prefilter_input_buffer.extend_from_slice(&[5, 6]);
        emu.audio_prefilter_buffer.extend_from_slice(&[7, 8]);
        emu.audio_accum_left = 9;
        emu.audio_accum_right = 10;
        emu.audio_accum_cycles = 11;
        emu.audio_delay_line.extend([12, 13]);
        emu.audio_input_history = 14;
        emu.audio_filter_history[0] = 15;
        emu.audio_post_history = 16;
        emu.audio_post_history2 = 17;
        emu.audio_last_nonzero_output = 18;
        emu.audio_final_filter_history[0] = 19;

        emu.set_audio_params_for_debug(
            "improved dead=0 icur=140 iprev=-20 pregain=142 pthr=2400 nthr=2340 pcnum=126 ncnum=124 pos_bias=67 neg_bias=75 cur=136 prev=-8 prev2=6 post_pos_bias=5 post_neg_bias=-7 sign_hyst=28 fcur=126 fprev=0 fprev2=4 fprev3=-8 fnonzero=2 total_rmse=246.133884 objective_total=246.678884 total_peak_overage=109",
        )
        .expect("params should parse");

        assert_eq!(emu.audio_output_params.deadzone, 0);
        assert_eq!(emu.audio_output_params.input_filter_cur, 140);
        assert_eq!(emu.audio_output_params.input_filter_prev, -20);
        assert_eq!(emu.audio_output_params.prefilter_gain_num, 142);
        assert_eq!(emu.audio_output_params.compress_threshold_positive, 2_400);
        assert_eq!(emu.audio_output_params.compress_threshold_negative, 2_340);
        assert_eq!(emu.audio_output_params.compress_num_positive, 126);
        assert_eq!(emu.audio_output_params.compress_num_negative, 124);
        assert_eq!(emu.audio_output_params.positive_bias, 67);
        assert_eq!(emu.audio_output_params.negative_bias, 75);
        assert_eq!(emu.audio_output_params.post_filter_cur, 136);
        assert_eq!(emu.audio_output_params.post_filter_prev, -8);
        assert_eq!(emu.audio_output_params.post_filter_prev2, 6);
        assert_eq!(emu.audio_output_params.post_filter_positive_bias, 5);
        assert_eq!(emu.audio_output_params.post_filter_negative_bias, -7);
        assert_eq!(emu.audio_output_params.sign_hysteresis, 28);
        assert_eq!(emu.audio_output_params.final_filter_taps, [126, 0, 4, -8]);
        assert_eq!(emu.audio_output_params.final_nonzero_bias, 2);

        assert!(emu.audio_buffer.is_empty());
        assert!(emu.audio_pair_input_buffer.is_empty());
        assert!(emu.audio_prefilter_input_buffer.is_empty());
        assert!(emu.audio_prefilter_buffer.is_empty());
        assert_eq!(emu.audio_accum_left, 0);
        assert_eq!(emu.audio_accum_right, 0);
        assert_eq!(emu.audio_accum_cycles, 0);
        assert!(emu.audio_delay_line.is_empty());
        assert_eq!(emu.audio_input_history, 0);
        assert_eq!(emu.audio_filter_history, [0; AUDIO_OUTPUT_FILTER_TAPS.len() - 1]);
        assert_eq!(emu.audio_post_history, 0);
        assert_eq!(emu.audio_post_history2, 0);
        assert_eq!(emu.audio_last_nonzero_output, 0);
        assert_eq!(
            emu.audio_final_filter_history,
            [0; AUDIO_OUTPUT_FINAL_FILTER_TAPS.len() - 1]
        );
    }

    #[test]
    fn immediate_dma_keeps_visible_source_and_dest_registers() {
        let mut emu = Emulator::new();

        emu.ewram[0..4].copy_from_slice(&0x4433_2211u32.to_le_bytes());
        emu.io_write_u32_raw(0x0b0, 0x0200_0000);
        emu.io_write_u32_raw(0x0b4, 0x0200_0010);
        emu.io_write_u16_raw(0x0b8, 1);
        emu.write_io_u16(0x0ba, 0x8400);

        assert_eq!(emu.read_u32_mapped(0x0200_0010), 0x4433_2211);
        assert_eq!(emu.io_read_u32_raw(0x0b0), 0x0200_0000);
        assert_eq!(emu.io_read_u32_raw(0x0b4), 0x0200_0010);
        assert_eq!(emu.dma_src[0], 0x0200_0004);
        assert_eq!(emu.dma_dst[0], 0x0200_0014);
    }

    #[test]
    fn postflg_byte_write_does_not_enter_halt() {
        let mut emu = Emulator::new();

        emu.write_io_u8(0x300, 0x01);

        assert!(!emu.halted);
        assert!(!emu.stopped);
    }

    #[test]
    fn unmapped_io_region_does_not_mirror_haltcnt() {
        let mut emu = Emulator::new();

        emu.write_u16_mapped(0x04ff_f700, 0x0102);

        assert!(!emu.halted);
        assert!(!emu.stopped);
        assert_eq!(emu.io[REG_HALTCNT], 0);
        assert_eq!(emu.io[0x300], 0);
    }

    #[test]
    fn timer_reload_is_copied_on_start_and_overflow_raises_irq() {
        let mut emu = Emulator::new();

        emu.write_io_u16(REG_TM0CNT_L, 0xfffe);
        emu.write_io_u16(REG_TM0CNT_H, 0x00c0);
        assert_eq!(emu.read_u16_mapped(0x0400_0100), 0xfffe);

        emu.advance_time(2);

        assert_eq!(emu.read_u16_mapped(0x0400_0100), 0xfffe);
        assert_eq!(emu.io_read_u16_raw(REG_IF) & IRQ_TIMER0, IRQ_TIMER0);
    }

    #[test]
    fn count_up_timer_advances_from_previous_overflows() {
        let mut emu = Emulator::new();

        emu.write_io_u16(REG_TM0CNT_L, 0xffff);
        emu.write_io_u16(REG_TM0CNT_H, 0x0080);
        emu.write_io_u16(0x104, 0xfffe);
        emu.write_io_u16(0x106, 0x00c4);

        emu.advance_time(2);

        assert_eq!(emu.read_u16_mapped(0x0400_0100), 0xffff);
        assert_eq!(emu.read_u16_mapped(0x0400_0104), 0xfffe);
        assert_eq!(emu.io_read_u16_raw(REG_IF) & IRQ_TIMER1, IRQ_TIMER1);
    }

    #[test]
    fn interrupt_request_does_not_preseed_bios_intrwait_flags() {
        let mut emu = Emulator::new();
        emu.io_write_u16_raw(REG_IE, IRQ_VBLANK);

        emu.raise_interrupt(IRQ_VBLANK);

        assert_eq!(emu.io_read_u16_raw(REG_IF) & IRQ_VBLANK, IRQ_VBLANK);
        assert_eq!(emu.read_u16_mapped(0x03ff_fff8), 0);
    }
}
