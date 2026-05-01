mod cpu;
mod ppu;

use std::cell::UnsafeCell;
use std::sync::OnceLock;

use cpu::Cpu;
#[cfg(test)]
use cpu::Mode;

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

const CYCLES_PER_LINE: u32 = 1_232;
const HDRAW_CYCLES: u32 = 1_006;
const VISIBLE_LINES: u32 = 160;
const TOTAL_LINES: u32 = 228;
const FRAME_CYCLES: u32 = CYCLES_PER_LINE * TOTAL_LINES;

const REG_DISPCNT: usize = 0x000;
const REG_DISPSTAT: usize = 0x004;
const REG_VCOUNT: usize = 0x006;
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

const DMA_REG_BASES: [usize; 4] = [0x0b0, 0x0bc, 0x0c8, 0x0d4];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DmaTiming {
    Immediate,
    VBlank,
    HBlank,
    Special,
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
    audio_fraction: u64,
    keys: u16,
    frame_cycle: u32,
    halted: bool,
    stopped: bool,
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
            audio_fraction: 0,
            keys: 0,
            frame_cycle: 0,
            halted: false,
            stopped: false,
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
        self.audio_fraction = 0;
        self.keys = 0;
        self.frame_cycle = 0;
        self.halted = false;
        self.stopped = false;
        self.cpu.reset();

        self.io_write_u16_raw(REG_DISPCNT, 0x0080);
        self.io_write_u16_raw(REG_KEYCNT, 0);
        self.io_write_u16_raw(REG_IE, 0);
        self.io_write_u16_raw(REG_IF, 0);
        self.io_write_u16_raw(REG_WAITCNT, 0);
        self.io_write_u16_raw(REG_DISPSTAT, 0);
        self.io_write_u16_raw(0x088, 0x0200);
        self.io_write_u8_raw(REG_IME, 0);
        self.io_write_u8_raw(REG_HALTCNT, 0);
    }

    fn run_frame(&mut self) {
        self.audio_buffer.clear();
        self.run_cycles(FRAME_CYCLES);
        self.append_silence_for_frame();
    }

    fn run_cycles(&mut self, mut cycles: u32) {
        while cycles > 0 {
            if self.stopped {
                self.advance_time(cycles);
                break;
            }

            if self.halted {
                self.advance_time(cycles);
                break;
            }

            let used = self.step_cpu().max(1);
            self.advance_time(used);
            cycles = cycles.saturating_sub(used);
        }
    }

    fn append_silence_for_frame(&mut self) {
        let rate = self.audio_rate() as u64;
        self.audio_fraction += rate * FRAME_CYCLES as u64;
        let pairs = (self.audio_fraction / CPU_CLOCK_HZ as u64) as usize;
        self.audio_fraction %= CPU_CLOCK_HZ as u64;
        self.audio_buffer.resize(self.audio_buffer.len() + pairs * 2, 0);
    }

    fn audio_rate(&self) -> i32 {
        let sound_bias = self.io_read_u16_raw(0x088);
        if (sound_bias >> 14) & 0x3 != 0 {
            DOUBLE_AUDIO_RATE as i32
        } else {
            DEFAULT_AUDIO_RATE as i32
        }
    }

    fn advance_time(&mut self, mut cycles: u32) {
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
            let step = cycles.min(to_hblank.min(to_line_end).min(to_frame_end));

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
        }
    }

    fn on_line_start(&mut self, line: u32) {
        if line == VISIBLE_LINES {
            ppu::render_framebuffer(&self.io, &self.palette, &self.vram, &self.oam, &mut self.framebuffer);
            self.run_dma_timing(DmaTiming::VBlank);
            self.raise_interrupt(IRQ_VBLANK);
        }

        let dispstat = self.io_read_u16_raw(REG_DISPSTAT) & !0x0007;
        self.io_write_u16_raw(REG_DISPSTAT, dispstat);

        if line == self.vcount_target() as u32 {
            self.raise_interrupt(IRQ_VCOUNT);
        }

        self.poll_halt_wakeup();
    }

    fn on_hblank_start(&mut self, line: u32) {
        if line < VISIBLE_LINES {
            self.run_dma_timing(DmaTiming::HBlank);
            self.raise_interrupt(IRQ_HBLANK);
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

        if self.io_read_u16_raw(REG_IE) & irq_mask != 0 {
            let flags = self.read_u16_mapped(0x03ff_fff8);
            self.write_u16_mapped(0x03ff_fff8, flags | irq_mask);
        }

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

    fn io_write_u32_raw(&mut self, offset: usize, value: u32) {
        self.io_write_u16_raw(offset, value as u16);
        self.io_write_u16_raw(offset + 2, (value >> 16) as u16);
    }

    fn write_io_u8(&mut self, offset: usize, value: u8) {
        if offset >= IO_SIZE {
            return;
        }
        let reg = offset & !1;
        let old = if reg == REG_IF {
            self.io_read_u16_raw(REG_IF)
        } else {
            0
        };
        self.io[offset] = value;
        self.handle_io_write(reg, old);
    }

    fn write_io_u16(&mut self, offset: usize, value: u16) {
        if offset + 1 >= IO_SIZE {
            return;
        }
        let old = if offset == REG_IF {
            self.io_read_u16_raw(REG_IF)
        } else {
            0
        };
        self.io[offset] = value as u8;
        self.io[offset + 1] = (value >> 8) as u8;
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
            _ if offset == REG_HALTCNT || offset + 1 == REG_HALTCNT => {
                let value = self.io[REG_HALTCNT];
                if value & 0x80 == 0 {
                    self.halted = true;
                } else {
                    self.stopped = true;
                }
            }
            _ if DMA_REG_BASES
                .iter()
                .any(|base| offset == base + 0x0a || offset == base + 0x0b) =>
            {
                let channel = DMA_REG_BASES
                    .iter()
                    .position(|base| offset == base + 0x0a || offset == base + 0x0b)
                    .unwrap_or(0);
                self.handle_dma_control_write(channel);
            }
            _ => {}
        }
    }

    fn handle_dma_control_write(&mut self, channel: usize) {
        let base = DMA_REG_BASES[channel];
        let control = self.io_read_u16_raw(base + 0x0a);
        if control & (1 << 15) == 0 {
            return;
        }
        if self.dma_timing(channel) == DmaTiming::Immediate {
            self.run_dma_channel(channel);
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
        let base = DMA_REG_BASES[channel];
        let mut src = self.io_read_u32_raw(base);
        let mut dst = self.io_read_u32_raw(base + 0x04);
        let control = self.io_read_u16_raw(base + 0x0a);
        let mut count = self.io_read_u16_raw(base + 0x08) as u32;
        let word = control & (1 << 10) != 0;
        let repeat = control & (1 << 9) != 0 && self.dma_timing(channel) != DmaTiming::Immediate;
        let dst_mode = (control >> 5) & 0x3;
        let src_mode = (control >> 7) & 0x3;

        if count == 0 {
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
                0 | 3 => dst.wrapping_add(transfer_size),
                1 => dst.wrapping_sub(transfer_size),
                2 => dst,
                _ => dst,
            };
        }

        if control & (1 << 14) != 0 {
            self.raise_interrupt(1 << (8 + channel));
        }

        self.io_write_u32_raw(base, src);
        if dst_mode != 3 || !repeat {
            self.io_write_u32_raw(base + 0x04, dst);
        }

        if !repeat {
            self.io_write_u16_raw(base + 0x0a, control & !(1 << 15));
        }
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
                let offset = addr as usize & (IO_SIZE - 1);
                self.io_read_u8(offset)
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
                let offset = addr as usize & (IO_SIZE - 1);
                self.write_io_u8(offset, value);
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
                let offset = addr as usize & (IO_SIZE - 1) & !1;
                self.write_io_u16(offset, value);
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
                let offset = addr as usize & (IO_SIZE - 1) & !1;
                self.write_io_u32(offset, value);
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

    pub fn set_keys(&mut self, keys: u32) {
        self.inner.keys = (keys & 0x03ff) as u16;
    }

    pub fn run_frame(&mut self) {
        self.inner.run_frame();
    }

    pub fn framebuffer(&self) -> &[u32] {
        &self.inner.framebuffer
    }

    pub fn take_audio(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inner.audio_buffer)
    }

    pub fn audio_rate(&self) -> i32 {
        self.inner.audio_rate()
    }

    pub fn pc(&self) -> u32 {
        self.inner.cpu.pc()
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
}
