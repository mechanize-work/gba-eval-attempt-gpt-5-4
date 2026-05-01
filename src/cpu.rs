use crate::Emulator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    User = 0x10,
    Fiq = 0x11,
    Irq = 0x12,
    Supervisor = 0x13,
    Abort = 0x17,
    Undefined = 0x1b,
    System = 0x1f,
}

impl Mode {
    fn from_bits(bits: u8) -> Self {
        match bits & 0x1f {
            0x10 => Self::User,
            0x11 => Self::Fiq,
            0x12 => Self::Irq,
            0x13 => Self::Supervisor,
            0x17 => Self::Abort,
            0x1b => Self::Undefined,
            0x1f => Self::System,
            _ => Self::Supervisor,
        }
    }

    fn has_spsr(self) -> bool {
        !matches!(self, Self::User | Self::System)
    }
}

pub(crate) struct Cpu {
    regs: [u32; 16],
    cpsr: u32,
    spsr_fiq: u32,
    spsr_irq: u32,
    spsr_svc: u32,
    spsr_abt: u32,
    spsr_und: u32,
    bank_usr_r8_r12: [u32; 5],
    bank_usr_r13_r14: [u32; 2],
    bank_fiq_r8_r12: [u32; 5],
    bank_fiq_r13_r14: [u32; 2],
    bank_irq_r13_r14: [u32; 2],
    bank_svc_r13_r14: [u32; 2],
    bank_abt_r13_r14: [u32; 2],
    bank_und_r13_r14: [u32; 2],
    thumb_bl_prefix: Option<u32>,
}

impl Cpu {
    pub(crate) fn new() -> Self {
        let mut cpu = Self {
            regs: [0; 16],
            cpsr: 0xd3,
            spsr_fiq: 0,
            spsr_irq: 0,
            spsr_svc: 0,
            spsr_abt: 0,
            spsr_und: 0,
            bank_usr_r8_r12: [0; 5],
            bank_usr_r13_r14: [0; 2],
            bank_fiq_r8_r12: [0; 5],
            bank_fiq_r13_r14: [0; 2],
            bank_irq_r13_r14: [0; 2],
            bank_svc_r13_r14: [0; 2],
            bank_abt_r13_r14: [0; 2],
            bank_und_r13_r14: [0; 2],
            thumb_bl_prefix: None,
        };
        cpu.reset();
        cpu
    }

    pub(crate) fn reset(&mut self) {
        self.regs = [0; 16];
        self.cpsr = 0xd3;
        self.spsr_fiq = 0;
        self.spsr_irq = 0;
        self.spsr_svc = 0;
        self.spsr_abt = 0;
        self.spsr_und = 0;
        self.bank_usr_r8_r12 = [0; 5];
        self.bank_usr_r13_r14 = [0; 2];
        self.bank_fiq_r8_r12 = [0; 5];
        self.bank_fiq_r13_r14 = [0; 2];
        self.bank_irq_r13_r14 = [0; 2];
        self.bank_svc_r13_r14 = [0; 2];
        self.bank_abt_r13_r14 = [0; 2];
        self.bank_und_r13_r14 = [0; 2];
        self.thumb_bl_prefix = None;
    }

    pub(crate) fn pc(&self) -> u32 {
        self.regs[15]
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn cpsr(&self) -> u32 {
        self.cpsr
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn registers(&self) -> [u32; 16] {
        self.regs
    }

    #[cfg(test)]
    pub(crate) fn debug_set_state(&mut self, cpsr: u32, pc: u32) {
        self.set_cpsr_raw(cpsr);
        self.regs[15] = pc;
    }

    #[cfg(test)]
    pub(crate) fn debug_reg(&self, index: usize) -> u32 {
        self.regs[index]
    }

    pub(crate) fn mode(&self) -> Mode {
        Mode::from_bits(self.cpsr as u8)
    }

    pub(crate) fn thumb(&self) -> bool {
        self.cpsr & (1 << 5) != 0
    }

    pub(crate) fn irq_disabled(&self) -> bool {
        self.cpsr & (1 << 7) != 0
    }

    fn carry(&self) -> bool {
        self.cpsr & (1 << 29) != 0
    }

    fn overflow(&self) -> bool {
        self.cpsr & (1 << 28) != 0
    }

    fn negative(&self) -> bool {
        self.cpsr & (1 << 31) != 0
    }

    fn zero(&self) -> bool {
        self.cpsr & (1 << 30) != 0
    }

    fn set_nz(&mut self, value: u32) {
        if value & 0x8000_0000 != 0 {
            self.cpsr |= 1 << 31;
        } else {
            self.cpsr &= !(1 << 31);
        }
        if value == 0 {
            self.cpsr |= 1 << 30;
        } else {
            self.cpsr &= !(1 << 30);
        }
    }

    fn set_carry(&mut self, carry: bool) {
        if carry {
            self.cpsr |= 1 << 29;
        } else {
            self.cpsr &= !(1 << 29);
        }
    }

    fn set_overflow(&mut self, overflow: bool) {
        if overflow {
            self.cpsr |= 1 << 28;
        } else {
            self.cpsr &= !(1 << 28);
        }
    }

    fn read_reg(&self, index: usize, current_pc: u32, thumb_state: bool) -> u32 {
        if index == 15 {
            current_pc.wrapping_add(if thumb_state { 4 } else { 8 })
        } else {
            self.regs[index]
        }
    }

    fn read_reg_for_store_arm(&self, index: usize, current_pc: u32) -> u32 {
        if index == 15 {
            current_pc.wrapping_add(12)
        } else {
            self.regs[index]
        }
    }

    fn write_reg(&mut self, index: usize, value: u32) {
        if index != 15 {
            self.regs[index] = value;
        }
    }

    fn load_pc_arm(&mut self, value: u32) {
        self.regs[15] = value & !3;
        self.thumb_bl_prefix = None;
    }

    fn load_pc_thumb(&mut self, value: u32) {
        self.regs[15] = value & !1;
        self.thumb_bl_prefix = None;
    }

    fn branch_exchange(&mut self, value: u32) {
        if value & 1 != 0 {
            self.cpsr |= 1 << 5;
            self.regs[15] = value & !1;
        } else {
            self.cpsr &= !(1 << 5);
            self.regs[15] = value & !3;
        }
        self.thumb_bl_prefix = None;
    }

    fn save_banked(&mut self, mode: Mode) {
        match mode {
            Mode::User | Mode::System => {
                self.bank_usr_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_usr_r13_r14[0] = self.regs[13];
                self.bank_usr_r13_r14[1] = self.regs[14];
            }
            Mode::Fiq => {
                self.bank_fiq_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_fiq_r13_r14[0] = self.regs[13];
                self.bank_fiq_r13_r14[1] = self.regs[14];
            }
            Mode::Irq => {
                self.bank_usr_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_irq_r13_r14[0] = self.regs[13];
                self.bank_irq_r13_r14[1] = self.regs[14];
            }
            Mode::Supervisor => {
                self.bank_usr_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_svc_r13_r14[0] = self.regs[13];
                self.bank_svc_r13_r14[1] = self.regs[14];
            }
            Mode::Abort => {
                self.bank_usr_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_abt_r13_r14[0] = self.regs[13];
                self.bank_abt_r13_r14[1] = self.regs[14];
            }
            Mode::Undefined => {
                self.bank_usr_r8_r12.copy_from_slice(&self.regs[8..13]);
                self.bank_und_r13_r14[0] = self.regs[13];
                self.bank_und_r13_r14[1] = self.regs[14];
            }
        }
    }

    fn load_banked(&mut self, mode: Mode) {
        match mode {
            Mode::User | Mode::System => {
                self.regs[8..13].copy_from_slice(&self.bank_usr_r8_r12);
                self.regs[13] = self.bank_usr_r13_r14[0];
                self.regs[14] = self.bank_usr_r13_r14[1];
            }
            Mode::Fiq => {
                self.regs[8..13].copy_from_slice(&self.bank_fiq_r8_r12);
                self.regs[13] = self.bank_fiq_r13_r14[0];
                self.regs[14] = self.bank_fiq_r13_r14[1];
            }
            Mode::Irq => {
                self.regs[8..13].copy_from_slice(&self.bank_usr_r8_r12);
                self.regs[13] = self.bank_irq_r13_r14[0];
                self.regs[14] = self.bank_irq_r13_r14[1];
            }
            Mode::Supervisor => {
                self.regs[8..13].copy_from_slice(&self.bank_usr_r8_r12);
                self.regs[13] = self.bank_svc_r13_r14[0];
                self.regs[14] = self.bank_svc_r13_r14[1];
            }
            Mode::Abort => {
                self.regs[8..13].copy_from_slice(&self.bank_usr_r8_r12);
                self.regs[13] = self.bank_abt_r13_r14[0];
                self.regs[14] = self.bank_abt_r13_r14[1];
            }
            Mode::Undefined => {
                self.regs[8..13].copy_from_slice(&self.bank_usr_r8_r12);
                self.regs[13] = self.bank_und_r13_r14[0];
                self.regs[14] = self.bank_und_r13_r14[1];
            }
        }
    }

    fn set_cpsr_raw(&mut self, value: u32) {
        let old_mode = self.mode();
        let new_mode = Mode::from_bits(value as u8);
        if old_mode != new_mode {
            self.save_banked(old_mode);
        }
        self.cpsr = value;
        if old_mode != new_mode {
            self.load_banked(new_mode);
        }
    }

    fn update_cpsr_by_mask(&mut self, value: u32, field_mask: u32, privileged: bool) {
        let mut next = self.cpsr;
        if field_mask & 0x8 != 0 {
            next = (next & 0x00ff_ffff) | (value & 0xff00_0000);
        }
        if privileged {
            if field_mask & 0x4 != 0 {
                next = (next & 0xff00_ffff) | (value & 0x00ff_0000);
            }
            if field_mask & 0x2 != 0 {
                next = (next & 0xffff_00ff) | (value & 0x0000_ff00);
            }
            if field_mask & 0x1 != 0 {
                next = (next & 0xffff_ff00) | (value & 0x0000_00ff);
            }
        }
        self.set_cpsr_raw(next);
    }

    fn spsr(&self, mode: Mode) -> Option<u32> {
        match mode {
            Mode::Fiq => Some(self.spsr_fiq),
            Mode::Irq => Some(self.spsr_irq),
            Mode::Supervisor => Some(self.spsr_svc),
            Mode::Abort => Some(self.spsr_abt),
            Mode::Undefined => Some(self.spsr_und),
            Mode::User | Mode::System => None,
        }
    }

    fn current_spsr(&self) -> Option<u32> {
        self.spsr(self.mode())
    }

    fn set_spsr(&mut self, mode: Mode, value: u32) {
        match mode {
            Mode::Fiq => self.spsr_fiq = value,
            Mode::Irq => self.spsr_irq = value,
            Mode::Supervisor => self.spsr_svc = value,
            Mode::Abort => self.spsr_abt = value,
            Mode::Undefined => self.spsr_und = value,
            Mode::User | Mode::System => {}
        }
    }

    pub(crate) fn enter_exception(&mut self, mode: Mode, vector: u32, return_addr: u32) {
        let old_cpsr = self.cpsr;
        self.set_spsr(mode, old_cpsr);
        let next = (old_cpsr & 0xf000_0040) | (1 << 7) | (mode as u32);
        self.set_cpsr_raw(next);
        self.regs[14] = return_addr;
        self.regs[15] = vector;
        self.thumb_bl_prefix = None;
    }

    fn restore_cpsr_from_spsr(&mut self) {
        if let Some(value) = self.current_spsr() {
            self.set_cpsr_raw(value);
        }
    }
}

impl Emulator {
    pub(crate) fn step_cpu(&mut self) -> u32 {
        self.debug_instruction_count = self.debug_instruction_count.wrapping_add(1);
        if self.cpu.thumb() {
            self.step_thumb()
        } else {
            self.step_arm()
        }
    }

    fn step_arm(&mut self) -> u32 {
        let pc = self.cpu.pc();
        let instr = self.read_u32_mapped(pc);
        let next_pc = pc.wrapping_add(4);

        if !arm_condition_passed(&self.cpu, instr >> 28) {
            self.cpu.load_pc_arm(next_pc);
            return 1;
        }

        if instr & 0x0fff_fff0 == 0x012f_ff10 {
            let rm = (instr & 0xf) as usize;
            let value = self.cpu.read_reg(rm, pc, false);
            self.cpu.branch_exchange(value);
            return 3;
        }

        if instr & 0x0f80_00f0 == 0x0080_0090 {
            return self.exec_arm_mul_long(instr, pc);
        }

        if instr & 0x0fc0_00f0 == 0x0000_0090 {
            return self.exec_arm_mul(instr, pc);
        }

        if instr & 0x0fbf_0fff == 0x010f_0000 {
            let rd = ((instr >> 12) & 0xf) as usize;
            let spsr = instr & (1 << 22) != 0;
            let value = if spsr {
                self.cpu.current_spsr().unwrap_or(self.cpu.cpsr)
            } else {
                self.cpu.cpsr
            };
            self.cpu.write_reg(rd, value);
            self.cpu.load_pc_arm(next_pc);
            return 1;
        }

        if instr & 0x0db0_fff0 == 0x0120_f000 {
            let rm = (instr & 0xf) as usize;
            let value = self.cpu.read_reg(rm, pc, false);
            let spsr = instr & (1 << 22) != 0;
            let fields = (instr >> 16) & 0xf;
            if spsr {
                if self.cpu.mode().has_spsr() {
                    let mut cur = self.cpu.current_spsr().unwrap_or(0);
                    cur = apply_psr_mask(cur, value, fields);
                    self.cpu.set_spsr(self.cpu.mode(), cur);
                }
            } else {
                self.cpu
                    .update_cpsr_by_mask(value, fields, !matches!(self.cpu.mode(), Mode::User));
            }
            self.cpu.load_pc_arm(next_pc);
            return 1;
        }

        if instr & 0x0db0_f000 == 0x0320_f000 {
            let imm = arm_expand_immediate(instr, self.cpu.carry()).0;
            let spsr = instr & (1 << 22) != 0;
            let fields = (instr >> 16) & 0xf;
            if spsr {
                if self.cpu.mode().has_spsr() {
                    let mut cur = self.cpu.current_spsr().unwrap_or(0);
                    cur = apply_psr_mask(cur, imm, fields);
                    self.cpu.set_spsr(self.cpu.mode(), cur);
                }
            } else {
                self.cpu
                    .update_cpsr_by_mask(imm, fields, !matches!(self.cpu.mode(), Mode::User));
            }
            self.cpu.load_pc_arm(next_pc);
            return 1;
        }

        if instr & 0x0f00_0000 == 0x0f00_0000 {
            self.cpu.enter_exception(Mode::Supervisor, 0x08, pc.wrapping_add(4));
            return 3;
        }

        if instr & 0x0e00_0000 == 0x0a00_0000 {
            let offset = sign_extend((instr & 0x00ff_ffff) << 2, 26);
            if instr & (1 << 24) != 0 {
                self.cpu.write_reg(14, pc.wrapping_add(4));
            }
            self.cpu.load_pc_arm(pc.wrapping_add(8).wrapping_add(offset));
            return 3;
        }

        if instr & 0x0e00_0090 == 0x0000_0090 && instr & 0x0000_0060 != 0 {
            return self.exec_arm_halfword_transfer(instr, pc, next_pc);
        }

        if instr & 0x0e00_0000 == 0x0800_0000 {
            return self.exec_arm_block_transfer(instr, pc, next_pc);
        }

        if instr & 0x0c00_0000 == 0x0400_0000 {
            return self.exec_arm_single_transfer(instr, pc, next_pc);
        }

        if instr & 0x0c00_0000 == 0x0000_0000 {
            return self.exec_arm_data_processing(instr, pc, next_pc);
        }

        self.halted = true;
        1
    }

    fn exec_arm_mul(&mut self, instr: u32, pc: u32) -> u32 {
        let accumulate = instr & (1 << 21) != 0;
        let set_flags = instr & (1 << 20) != 0;
        let rd = ((instr >> 16) & 0xf) as usize;
        let rn = ((instr >> 12) & 0xf) as usize;
        let rs = ((instr >> 8) & 0xf) as usize;
        let rm = (instr & 0xf) as usize;

        let lhs = self.cpu.read_reg(rm, pc, false);
        let rhs = self.cpu.read_reg(rs, pc, false);
        let mut result = lhs.wrapping_mul(rhs);
        if accumulate {
            result = result.wrapping_add(self.cpu.read_reg(rn, pc, false));
        }
        self.cpu.write_reg(rd, result);
        if set_flags {
            self.cpu.set_nz(result);
        }
        self.cpu.load_pc_arm(pc.wrapping_add(4));
        2
    }

    fn exec_arm_mul_long(&mut self, instr: u32, pc: u32) -> u32 {
        let signed = instr & (1 << 22) != 0;
        let accumulate = instr & (1 << 21) != 0;
        let set_flags = instr & (1 << 20) != 0;
        let rd_hi = ((instr >> 16) & 0xf) as usize;
        let rd_lo = ((instr >> 12) & 0xf) as usize;
        let rs = ((instr >> 8) & 0xf) as usize;
        let rm = (instr & 0xf) as usize;

        let lhs = self.cpu.read_reg(rm, pc, false);
        let rhs = self.cpu.read_reg(rs, pc, false);
        let mut value = if signed {
            (lhs as i32 as i64).wrapping_mul(rhs as i32 as i64) as u64
        } else {
            (lhs as u64).wrapping_mul(rhs as u64)
        };
        if accumulate {
            value = value.wrapping_add(
                ((self.cpu.read_reg(rd_hi, pc, false) as u64) << 32)
                    | self.cpu.read_reg(rd_lo, pc, false) as u64,
            );
        }
        self.cpu.write_reg(rd_lo, value as u32);
        self.cpu.write_reg(rd_hi, (value >> 32) as u32);
        if set_flags {
            self.cpu.set_nz((value >> 32) as u32);
            if value == 0 {
                self.cpu.cpsr |= 1 << 30;
            } else {
                self.cpu.cpsr &= !(1 << 30);
            }
        }
        self.cpu.load_pc_arm(pc.wrapping_add(4));
        3
    }

    fn exec_arm_data_processing(&mut self, instr: u32, pc: u32, next_pc: u32) -> u32 {
        let opcode = (instr >> 21) & 0xf;
        let set_flags = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        let (operand2, shifter_carry) = if instr & (1 << 25) != 0 {
            arm_expand_immediate(instr, self.cpu.carry())
        } else {
            self.arm_shift_operand(instr, pc)
        };
        let rn_value = self.cpu.read_reg(rn, pc, false);

        let mut write_result = true;
        let mut carry = self.cpu.carry();
        let mut overflow = self.cpu.overflow();
        let result = match opcode {
            0x0 => {
                carry = shifter_carry;
                rn_value & operand2
            }
            0x1 => {
                carry = shifter_carry;
                rn_value ^ operand2
            }
            0x2 => {
                let (res, c, v) = sub_with_carry(rn_value, operand2, true);
                carry = c;
                overflow = v;
                res
            }
            0x3 => {
                let (res, c, v) = sub_with_carry(operand2, rn_value, true);
                carry = c;
                overflow = v;
                res
            }
            0x4 => {
                let (res, c, v) = add_with_carry(rn_value, operand2, false);
                carry = c;
                overflow = v;
                res
            }
            0x5 => {
                let (res, c, v) = add_with_carry(rn_value, operand2, self.cpu.carry());
                carry = c;
                overflow = v;
                res
            }
            0x6 => {
                let (res, c, v) = sub_with_carry(rn_value, operand2, self.cpu.carry());
                carry = c;
                overflow = v;
                res
            }
            0x7 => {
                let (res, c, v) = sub_with_carry(operand2, rn_value, self.cpu.carry());
                carry = c;
                overflow = v;
                res
            }
            0x8 => {
                carry = shifter_carry;
                write_result = false;
                rn_value & operand2
            }
            0x9 => {
                carry = shifter_carry;
                write_result = false;
                rn_value ^ operand2
            }
            0xa => {
                write_result = false;
                let (res, c, v) = sub_with_carry(rn_value, operand2, true);
                carry = c;
                overflow = v;
                res
            }
            0xb => {
                write_result = false;
                let (res, c, v) = add_with_carry(rn_value, operand2, false);
                carry = c;
                overflow = v;
                res
            }
            0xc => {
                carry = shifter_carry;
                rn_value | operand2
            }
            0xd => {
                carry = shifter_carry;
                operand2
            }
            0xe => {
                carry = shifter_carry;
                rn_value & !operand2
            }
            0xf => {
                carry = shifter_carry;
                !operand2
            }
            _ => 0,
        };

        if set_flags {
            self.cpu.set_nz(result);
            self.cpu.set_carry(carry);
            self.cpu.set_overflow(overflow);
        }

        if write_result {
            if rd == 15 {
                if set_flags && self.cpu.mode().has_spsr() {
                    self.cpu.restore_cpsr_from_spsr();
                }
                if self.cpu.thumb() {
                    self.cpu.load_pc_thumb(result);
                } else {
                    self.cpu.load_pc_arm(result);
                }
            } else {
                self.cpu.write_reg(rd, result);
                self.cpu.load_pc_arm(next_pc);
            }
        } else {
            self.cpu.load_pc_arm(next_pc);
        }

        1
    }

    fn arm_shift_operand(&self, instr: u32, pc: u32) -> (u32, bool) {
        let rm = (instr & 0xf) as usize;
        let value = self.cpu.read_reg(rm, pc, false);
        let shift_type = (instr >> 5) & 0x3;
        if instr & (1 << 4) == 0 {
            let amount = (instr >> 7) & 0x1f;
            shift(value, shift_type, amount, self.cpu.carry(), false)
        } else {
            let rs = ((instr >> 8) & 0xf) as usize;
            let amount = self.cpu.read_reg(rs, pc, false) & 0xff;
            shift(value, shift_type, amount, self.cpu.carry(), true)
        }
    }

    fn exec_arm_single_transfer(&mut self, instr: u32, pc: u32, next_pc: u32) -> u32 {
        let immediate = instr & (1 << 25) == 0;
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let byte = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        let base = self.cpu.read_reg(rn, pc, false);

        let offset = if immediate {
            instr & 0xfff
        } else {
            let rm = (instr & 0xf) as usize;
            let shift_type = (instr >> 5) & 0x3;
            let amount = (instr >> 7) & 0x1f;
            shift(
                self.cpu.read_reg(rm, pc, false),
                shift_type,
                amount,
                self.cpu.carry(),
                false,
            )
            .0
        };

        let offset_addr = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if pre { offset_addr } else { base };

        if load {
            let value = if byte {
                self.read_u8_mapped(addr) as u32
            } else {
                self.read_u32_mapped(addr)
            };
            if rd == 15 {
                self.cpu.load_pc_arm(value);
            } else {
                self.cpu.write_reg(rd, value);
                self.cpu.load_pc_arm(next_pc);
            }
        } else {
            let value = self.cpu.read_reg_for_store_arm(rd, pc);
            if byte {
                self.write_u8_mapped(addr, value as u8);
            } else {
                self.write_u32_mapped(addr, value);
            }
            self.cpu.load_pc_arm(next_pc);
        }

        if (!pre || writeback) && !(load && rn == rd) && rn != 15 {
            self.cpu.write_reg(rn, offset_addr);
        }

        3
    }

    fn exec_arm_halfword_transfer(&mut self, instr: u32, pc: u32, next_pc: u32) -> u32 {
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let immediate = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rd = ((instr >> 12) & 0xf) as usize;
        let op = (instr >> 5) & 0x3;
        let base = self.cpu.read_reg(rn, pc, false);

        let offset = if immediate {
            ((instr >> 8) & 0xf) << 4 | (instr & 0xf)
        } else {
            let rm = (instr & 0xf) as usize;
            self.cpu.read_reg(rm, pc, false)
        };

        let offset_addr = if up {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let addr = if pre { offset_addr } else { base };

        if load {
            let value = match op {
                1 => self.read_u16_aligned(addr) as u32,
                2 => sign_extend(self.read_u8_mapped(addr) as u32, 8),
                3 => {
                    if addr & 1 == 0 {
                        sign_extend(self.read_u16_aligned(addr) as u32, 16)
                    } else {
                        sign_extend(self.read_u8_mapped(addr) as u32, 8)
                    }
                }
                _ => 0,
            };
            if rd == 15 {
                self.cpu.load_pc_arm(value);
            } else {
                self.cpu.write_reg(rd, value);
                self.cpu.load_pc_arm(next_pc);
            }
        } else {
            if op == 1 {
                self.write_u16_mapped(addr, self.cpu.read_reg_for_store_arm(rd, pc) as u16);
            }
            self.cpu.load_pc_arm(next_pc);
        }

        if (!pre || writeback) && !(load && rn == rd) && rn != 15 {
            self.cpu.write_reg(rn, offset_addr);
        }

        3
    }

    fn exec_arm_block_transfer(&mut self, instr: u32, pc: u32, next_pc: u32) -> u32 {
        let pre = instr & (1 << 24) != 0;
        let up = instr & (1 << 23) != 0;
        let s = instr & (1 << 22) != 0;
        let writeback = instr & (1 << 21) != 0;
        let load = instr & (1 << 20) != 0;
        let rn = ((instr >> 16) & 0xf) as usize;
        let rlist = instr & 0xffff;
        let count = rlist.count_ones();

        if count == 0 {
            self.cpu.load_pc_arm(next_pc);
            return 1;
        }

        let base = self.cpu.read_reg(rn, pc, false);
        let total = count * 4;
        let mut addr = match (up, pre) {
            (true, false) => base,
            (true, true) => base.wrapping_add(4),
            (false, false) => base.wrapping_sub(total).wrapping_add(4),
            (false, true) => base.wrapping_sub(total),
        };
        let final_base = if up {
            base.wrapping_add(total)
        } else {
            base.wrapping_sub(total)
        };

        let mut loaded_pc = None;
        for reg in 0..16 {
            if rlist & (1 << reg) == 0 {
                continue;
            }
            if load {
                let value = self.read_u32_mapped(addr);
                if reg == 15 {
                    loaded_pc = Some(value);
                } else {
                    self.cpu.write_reg(reg as usize, value);
                }
            } else {
                let value = self.cpu.read_reg_for_store_arm(reg as usize, pc);
                self.write_u32_mapped(addr, value);
            }
            addr = addr.wrapping_add(4);
        }

        if writeback && !(load && (rlist & (1 << rn)) != 0) {
            self.cpu.write_reg(rn, final_base);
        }

        if let Some(value) = loaded_pc {
            if s && self.cpu.mode().has_spsr() {
                self.cpu.restore_cpsr_from_spsr();
            }
            if self.cpu.thumb() {
                self.cpu.load_pc_thumb(value);
            } else {
                self.cpu.load_pc_arm(value);
            }
        } else {
            self.cpu.load_pc_arm(next_pc);
        }

        1 + count
    }

    fn step_thumb(&mut self) -> u32 {
        let pc = self.cpu.pc();
        let instr = self.read_u16_mapped(pc) as u16;
        let next_pc = pc.wrapping_add(2);

        match instr & 0xf800 {
            0x0000 | 0x0800 | 0x1000 => {
                let op = ((instr >> 11) & 0x3) as u32;
                let amount = ((instr >> 6) & 0x1f) as u32;
                let rs = ((instr >> 3) & 0x7) as usize;
                let rd = (instr & 0x7) as usize;
                let value = self.cpu.read_reg(rs, pc, true);
                let (result, carry) = shift(value, op, amount, self.cpu.carry(), false);
                self.cpu.write_reg(rd, result);
                self.cpu.set_nz(result);
                self.cpu.set_carry(carry);
                self.cpu.load_pc_thumb(next_pc);
                return 1;
            }
            _ => {}
        }

        if instr & 0xf800 == 0x1800 {
            let immediate = instr & (1 << 10) != 0;
            let subtract = instr & (1 << 9) != 0;
            let operand = ((instr >> 6) & 0x7) as usize;
            let rs = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let lhs = self.cpu.read_reg(rs, pc, true);
            let rhs = if immediate {
                operand as u32
            } else {
                self.cpu.read_reg(operand, pc, true)
            };
            let (result, carry, overflow) = if subtract {
                sub_with_carry(lhs, rhs, true)
            } else {
                add_with_carry(lhs, rhs, false)
            };
            self.cpu.write_reg(rd, result);
            self.cpu.set_nz(result);
            self.cpu.set_carry(carry);
            self.cpu.set_overflow(overflow);
            self.cpu.load_pc_thumb(next_pc);
            return 1;
        }

        if instr & 0xe000 == 0x2000 {
            let op = (instr >> 11) & 0x3;
            let rd = ((instr >> 8) & 0x7) as usize;
            let imm = (instr & 0xff) as u32;
            match op {
                0 => {
                    self.cpu.write_reg(rd, imm);
                    self.cpu.set_nz(imm);
                }
                1 => {
                    let (result, carry, overflow) =
                        sub_with_carry(self.cpu.read_reg(rd, pc, true), imm, true);
                    self.cpu.set_nz(result);
                    self.cpu.set_carry(carry);
                    self.cpu.set_overflow(overflow);
                }
                2 => {
                    let (result, carry, overflow) =
                        add_with_carry(self.cpu.read_reg(rd, pc, true), imm, false);
                    self.cpu.write_reg(rd, result);
                    self.cpu.set_nz(result);
                    self.cpu.set_carry(carry);
                    self.cpu.set_overflow(overflow);
                }
                3 => {
                    let (result, carry, overflow) =
                        sub_with_carry(self.cpu.read_reg(rd, pc, true), imm, true);
                    self.cpu.write_reg(rd, result);
                    self.cpu.set_nz(result);
                    self.cpu.set_carry(carry);
                    self.cpu.set_overflow(overflow);
                }
                _ => {}
            }
            self.cpu.load_pc_thumb(next_pc);
            return 1;
        }

        if instr & 0xfc00 == 0x4000 {
            let op = (instr >> 6) & 0xf;
            let rs = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let lhs = self.cpu.read_reg(rd, pc, true);
            let rhs = self.cpu.read_reg(rs, pc, true);
            let mut write = true;
            let mut result = lhs;
            let mut carry = self.cpu.carry();
            let mut overflow = self.cpu.overflow();
            match op {
                0x0 => result &= rhs,
                0x1 => result ^= rhs,
                0x2 => {
                    let (v, c) = shift(lhs, 0, rhs & 0xff, self.cpu.carry(), true);
                    result = v;
                    carry = c;
                }
                0x3 => {
                    let (v, c) = shift(lhs, 1, rhs & 0xff, self.cpu.carry(), true);
                    result = v;
                    carry = c;
                }
                0x4 => {
                    let (v, c) = shift(lhs, 2, rhs & 0xff, self.cpu.carry(), true);
                    result = v;
                    carry = c;
                }
                0x5 => {
                    let (v, c, o) = add_with_carry(lhs, rhs, self.cpu.carry());
                    result = v;
                    carry = c;
                    overflow = o;
                }
                0x6 => {
                    let (v, c, o) = sub_with_carry(lhs, rhs, self.cpu.carry());
                    result = v;
                    carry = c;
                    overflow = o;
                }
                0x7 => {
                    let (v, c) = shift(lhs, 3, rhs & 0xff, self.cpu.carry(), true);
                    result = v;
                    carry = c;
                }
                0x8 => {
                    write = false;
                    result = lhs & rhs;
                }
                0x9 => {
                    let (v, c, o) = sub_with_carry(0, rhs, true);
                    result = v;
                    carry = c;
                    overflow = o;
                }
                0xa => {
                    write = false;
                    let (v, c, o) = sub_with_carry(lhs, rhs, true);
                    result = v;
                    carry = c;
                    overflow = o;
                }
                0xb => {
                    write = false;
                    let (v, c, o) = add_with_carry(lhs, rhs, false);
                    result = v;
                    carry = c;
                    overflow = o;
                }
                0xc => result |= rhs,
                0xd => result = lhs.wrapping_mul(rhs),
                0xe => result = lhs & !rhs,
                0xf => result = !rhs,
                _ => {}
            }
            if write {
                self.cpu.write_reg(rd, result);
            }
            self.cpu.set_nz(result);
            if matches!(op, 0x2..=0x7 | 0x9 | 0xa | 0xb) {
                self.cpu.set_carry(carry);
            }
            if matches!(op, 0x5 | 0x6 | 0x9 | 0xa | 0xb) {
                self.cpu.set_overflow(overflow);
            }
            self.cpu.load_pc_thumb(next_pc);
            return 1;
        }

        if instr & 0xfc00 == 0x4400 {
            let op = (instr >> 8) & 0x3;
            let rs = (((instr >> 3) & 0x7) | ((instr >> 3) & 0x8)) as usize;
            let rd = ((instr & 0x7) | ((instr >> 4) & 0x8)) as usize;
            let rhs = self.cpu.read_reg(rs, pc, true);
            match op {
                0 => {
                    let value = self.cpu.read_reg(rd, pc, true).wrapping_add(rhs);
                    if rd == 15 {
                        self.cpu.load_pc_thumb(value);
                    } else {
                        self.cpu.write_reg(rd, value);
                        self.cpu.load_pc_thumb(next_pc);
                    }
                }
                1 => {
                    let lhs = self.cpu.read_reg(rd, pc, true);
                    let (result, carry, overflow) = sub_with_carry(lhs, rhs, true);
                    self.cpu.set_nz(result);
                    self.cpu.set_carry(carry);
                    self.cpu.set_overflow(overflow);
                    self.cpu.load_pc_thumb(next_pc);
                }
                2 => {
                    if rd == 15 {
                        self.cpu.load_pc_thumb(rhs);
                    } else {
                        self.cpu.write_reg(rd, rhs);
                        self.cpu.load_pc_thumb(next_pc);
                    }
                }
                3 => {
                    self.cpu.branch_exchange(rhs);
                }
                _ => {}
            }
            return 3;
        }

        if instr & 0xf800 == 0x4800 {
            let rd = ((instr >> 8) & 0x7) as usize;
            let addr = (pc.wrapping_add(4) & !2).wrapping_add(((instr & 0xff) as u32) << 2);
            let value = self.read_u32_mapped(addr);
            self.cpu.write_reg(rd, value);
            self.cpu.load_pc_thumb(next_pc);
            return 3;
        }

        if instr & 0xf200 == 0x5000 {
            let op = (instr >> 10) & 0x3;
            let ro = ((instr >> 6) & 0x7) as usize;
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let addr = self
                .cpu
                .read_reg(rb, pc, true)
                .wrapping_add(self.cpu.read_reg(ro, pc, true));
            match op {
                0 => self.write_u32_mapped(addr, self.cpu.read_reg(rd, pc, true)),
                1 => self.write_u8_mapped(addr, self.cpu.read_reg(rd, pc, true) as u8),
                2 => {
                    let value = self.read_u32_mapped(addr);
                    self.cpu.write_reg(rd, value);
                }
                3 => {
                    let value = self.read_u8_mapped(addr) as u32;
                    self.cpu.write_reg(rd, value);
                }
                _ => {}
            }
            self.cpu.load_pc_thumb(next_pc);
            return 2;
        }

        if instr & 0xf200 == 0x5200 {
            let op = (instr >> 10) & 0x3;
            let ro = ((instr >> 6) & 0x7) as usize;
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let addr = self
                .cpu
                .read_reg(rb, pc, true)
                .wrapping_add(self.cpu.read_reg(ro, pc, true));
            let value = match op {
                0 => {
                    self.write_u16_mapped(addr, self.cpu.read_reg(rd, pc, true) as u16);
                    None
                }
                1 => Some(sign_extend(self.read_u8_mapped(addr) as u32, 8)),
                2 => Some(self.read_u16_aligned(addr) as u32),
                3 => Some(if addr & 1 == 0 {
                    sign_extend(self.read_u16_aligned(addr) as u32, 16)
                } else {
                    sign_extend(self.read_u8_mapped(addr) as u32, 8)
                }),
                _ => None,
            };
            if let Some(v) = value {
                self.cpu.write_reg(rd, v);
            }
            self.cpu.load_pc_thumb(next_pc);
            return 2;
        }

        if instr & 0xe000 == 0x6000 {
            let op = (instr >> 11) & 0x3;
            let imm5 = ((instr >> 6) & 0x1f) as u32;
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let base = self.cpu.read_reg(rb, pc, true);
            let word = op < 2;
            let load = op & 1 != 0;
            let addr = if word {
                base.wrapping_add(imm5 << 2)
            } else {
                base.wrapping_add(imm5)
            };
            if load {
                let value = if word {
                    self.read_u32_mapped(addr)
                } else {
                    self.read_u8_mapped(addr) as u32
                };
                self.cpu.write_reg(rd, value);
            } else if word {
                self.write_u32_mapped(addr, self.cpu.read_reg(rd, pc, true));
            } else {
                self.write_u8_mapped(addr, self.cpu.read_reg(rd, pc, true) as u8);
            }
            self.cpu.load_pc_thumb(next_pc);
            return 2;
        }

        if instr & 0xf000 == 0x8000 {
            let load = instr & (1 << 11) != 0;
            let imm5 = ((instr >> 6) & 0x1f) as u32;
            let rb = ((instr >> 3) & 0x7) as usize;
            let rd = (instr & 0x7) as usize;
            let addr = self
                .cpu
                .read_reg(rb, pc, true)
                .wrapping_add(imm5 << 1);
            if load {
                let value = self.read_u16_aligned(addr) as u32;
                self.cpu.write_reg(rd, value);
            } else {
                self.write_u16_mapped(addr, self.cpu.read_reg(rd, pc, true) as u16);
            }
            self.cpu.load_pc_thumb(next_pc);
            return 2;
        }

        if instr & 0xf000 == 0x9000 {
            let load = instr & (1 << 11) != 0;
            let rd = ((instr >> 8) & 0x7) as usize;
            let addr = self
                .cpu
                .read_reg(13, pc, true)
                .wrapping_add(((instr & 0xff) as u32) << 2);
            if load {
                let value = self.read_u32_mapped(addr);
                self.cpu.write_reg(rd, value);
            } else {
                self.write_u32_mapped(addr, self.cpu.read_reg(rd, pc, true));
            }
            self.cpu.load_pc_thumb(next_pc);
            return 2;
        }

        if instr & 0xf000 == 0xa000 {
            let rd = ((instr >> 8) & 0x7) as usize;
            let imm = ((instr & 0xff) as u32) << 2;
            let value = if instr & (1 << 11) != 0 {
                self.cpu.read_reg(13, pc, true).wrapping_add(imm)
            } else {
                (pc.wrapping_add(4) & !2).wrapping_add(imm)
            };
            self.cpu.write_reg(rd, value);
            self.cpu.load_pc_thumb(next_pc);
            return 1;
        }

        if instr & 0xff00 == 0xb000 {
            let imm = ((instr & 0x7f) as u32) << 2;
            let sp = self.cpu.read_reg(13, pc, true);
            let value = if instr & (1 << 7) != 0 {
                sp.wrapping_sub(imm)
            } else {
                sp.wrapping_add(imm)
            };
            self.cpu.write_reg(13, value);
            self.cpu.load_pc_thumb(next_pc);
            return 1;
        }

        if instr & 0xf600 == 0xb400 {
            let load = instr & (1 << 11) != 0;
            let extra = instr & (1 << 8) != 0;
            let rlist = instr & 0xff;
            let count = rlist.count_ones() + u32::from(extra);
            let mut sp = self.cpu.read_reg(13, pc, true);
            if !load {
                sp = sp.wrapping_sub(count * 4);
                let mut addr = sp;
                for reg in 0..8 {
                    if rlist & (1 << reg) != 0 {
                        self.write_u32_mapped(addr, self.cpu.read_reg(reg as usize, pc, true));
                        addr = addr.wrapping_add(4);
                    }
                }
                if extra {
                    self.write_u32_mapped(addr, self.cpu.read_reg(14, pc, true));
                }
            } else {
                let mut addr = sp;
                for reg in 0..8 {
                    if rlist & (1 << reg) != 0 {
                        let value = self.read_u32_mapped(addr);
                        self.cpu.write_reg(reg as usize, value);
                        addr = addr.wrapping_add(4);
                    }
                }
                if extra {
                    let value = self.read_u32_mapped(addr);
                    self.cpu.load_pc_thumb(value);
                } else {
                    self.cpu.load_pc_thumb(next_pc);
                }
                sp = sp.wrapping_add(count * 4);
                self.cpu.write_reg(13, sp);
                return 1 + count;
            }
            self.cpu.write_reg(13, sp);
            self.cpu.load_pc_thumb(next_pc);
            return 1 + count;
        }

        if instr & 0xf000 == 0xc000 {
            let load = instr & (1 << 11) != 0;
            let rb = ((instr >> 8) & 0x7) as usize;
            let rlist = instr & 0xff;
            let count = rlist.count_ones();
            let base = self.cpu.read_reg(rb, pc, true);
            let final_base = base.wrapping_add(count * 4);
            let rb_in_list = rlist & (1 << rb) != 0;
            let mut addr = base;
            for reg in 0..8 {
                if rlist & (1 << reg) == 0 {
                    continue;
                }
                if load {
                    let value = self.read_u32_mapped(addr);
                    self.cpu.write_reg(reg as usize, value);
                } else {
                    self.write_u32_mapped(addr, self.cpu.read_reg(reg as usize, pc, true));
                }
                addr = addr.wrapping_add(4);
            }
            if !load || !rb_in_list {
                self.cpu.write_reg(rb, final_base);
            }
            self.cpu.load_pc_thumb(next_pc);
            return 1 + count;
        }

        if instr & 0xf000 == 0xd000 {
            let cond = (instr >> 8) & 0xf;
            if cond == 0xf {
                self.cpu.enter_exception(Mode::Supervisor, 0x08, pc.wrapping_add(2));
                return 3;
            }
            self.cpu.load_pc_thumb(next_pc);
            if thumb_condition_passed(&self.cpu, cond as u32) {
                let offset = sign_extend(((instr & 0xff) as u32) << 1, 9);
                self.cpu.load_pc_thumb(pc.wrapping_add(4).wrapping_add(offset));
                return 3;
            }
            return 1;
        }

        if instr & 0xf800 == 0xe000 {
            let offset = sign_extend(((instr & 0x07ff) as u32) << 1, 12);
            self.cpu.load_pc_thumb(pc.wrapping_add(4).wrapping_add(offset));
            return 3;
        }

        if instr & 0xf800 == 0xf000 {
            let offset = sign_extend(((instr & 0x07ff) as u32) << 12, 23);
            self.cpu.thumb_bl_prefix = Some(pc.wrapping_add(4).wrapping_add(offset));
            // Keep the BL prefix alive for the immediately following low-halfword.
            self.cpu.regs[15] = next_pc & !1;
            return 1;
        }

        if instr & 0xf800 == 0xf800 {
            let lower = ((instr & 0x07ff) as u32) << 1;
            let prefix = self.cpu.thumb_bl_prefix.unwrap_or(pc.wrapping_add(4));
            let return_addr = next_pc | 1;
            self.cpu.write_reg(14, return_addr);
            self.cpu.load_pc_thumb(prefix.wrapping_add(lower));
            return 3;
        }

        self.halted = true;
        1
    }
}

fn arm_condition_passed(cpu: &Cpu, cond: u32) -> bool {
    match cond & 0xf {
        0x0 => cpu.zero(),
        0x1 => !cpu.zero(),
        0x2 => cpu.carry(),
        0x3 => !cpu.carry(),
        0x4 => cpu.negative(),
        0x5 => !cpu.negative(),
        0x6 => cpu.overflow(),
        0x7 => !cpu.overflow(),
        0x8 => cpu.carry() && !cpu.zero(),
        0x9 => !cpu.carry() || cpu.zero(),
        0xa => cpu.negative() == cpu.overflow(),
        0xb => cpu.negative() != cpu.overflow(),
        0xc => !cpu.zero() && (cpu.negative() == cpu.overflow()),
        0xd => cpu.zero() || (cpu.negative() != cpu.overflow()),
        0xe => true,
        _ => false,
    }
}

fn thumb_condition_passed(cpu: &Cpu, cond: u32) -> bool {
    arm_condition_passed(cpu, cond)
}

fn arm_expand_immediate(instr: u32, carry_in: bool) -> (u32, bool) {
    let imm = instr & 0xff;
    let rotate = ((instr >> 8) & 0xf) * 2;
    if rotate == 0 {
        (imm, carry_in)
    } else {
        let value = imm.rotate_right(rotate);
        (value, value & 0x8000_0000 != 0)
    }
}

fn sign_extend(value: u32, bits: u32) -> u32 {
    let shift = 32 - bits;
    ((value << shift) as i32 >> shift) as u32
}

fn add_with_carry(lhs: u32, rhs: u32, carry_in: bool) -> (u32, bool, bool) {
    let carry = u64::from(carry_in);
    let sum = lhs as u64 + rhs as u64 + carry;
    let result = sum as u32;
    let carry_out = sum >> 32 != 0;
    let overflow = ((lhs ^ result) & (rhs ^ result) & 0x8000_0000) != 0;
    (result, carry_out, overflow)
}

fn sub_with_carry(lhs: u32, rhs: u32, carry_in: bool) -> (u32, bool, bool) {
    add_with_carry(lhs, !rhs, carry_in)
}

fn shift(value: u32, shift_type: u32, amount: u32, carry_in: bool, register: bool) -> (u32, bool) {
    match shift_type & 0x3 {
        0 => {
            if amount == 0 {
                (value, carry_in)
            } else if amount < 32 {
                (value << amount, (value >> (32 - amount)) & 1 != 0)
            } else if amount == 32 {
                (0, value & 1 != 0)
            } else {
                (0, false)
            }
        }
        1 => {
            if amount == 0 {
                if register {
                    (value, carry_in)
                } else {
                    (0, value & 0x8000_0000 != 0)
                }
            } else if amount < 32 {
                (value >> amount, (value >> (amount - 1)) & 1 != 0)
            } else if amount == 32 {
                (0, value & 0x8000_0000 != 0)
            } else {
                (0, false)
            }
        }
        2 => {
            if amount == 0 {
                if register {
                    (value, carry_in)
                } else if value & 0x8000_0000 != 0 {
                    (u32::MAX, true)
                } else {
                    (0, false)
                }
            } else if amount < 32 {
                (
                    ((value as i32) >> amount) as u32,
                    (value >> (amount - 1)) & 1 != 0,
                )
            } else if value & 0x8000_0000 != 0 {
                (u32::MAX, true)
            } else {
                (0, false)
            }
        }
        _ => {
            if amount == 0 {
                if register {
                    (value, carry_in)
                } else {
                    (((carry_in as u32) << 31) | (value >> 1), value & 1 != 0)
                }
            } else {
                let rot = amount & 31;
                if rot == 0 {
                    (value, value & 0x8000_0000 != 0)
                } else {
                    let result = value.rotate_right(rot);
                    (result, (value >> (rot - 1)) & 1 != 0)
                }
            }
        }
    }
}

fn apply_psr_mask(current: u32, value: u32, field_mask: u32) -> u32 {
    let mut next = current;
    if field_mask & 0x8 != 0 {
        next = (next & 0x00ff_ffff) | (value & 0xff00_0000);
    }
    if field_mask & 0x4 != 0 {
        next = (next & 0xff00_ffff) | (value & 0x00ff_0000);
    }
    if field_mask & 0x2 != 0 {
        next = (next & 0xffff_00ff) | (value & 0x0000_ff00);
    }
    if field_mask & 0x1 != 0 {
        next = (next & 0xffff_ff00) | (value & 0x0000_00ff);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::Mode;
    use crate::Emulator;

    fn thumb_emu(pc: u32) -> Emulator {
        let mut emu = Emulator::new();
        emu.cpu.set_cpsr_raw((Mode::System as u32) | (1 << 5));
        emu.cpu.regs[15] = pc;
        emu
    }

    fn arm_emu(pc: u32, mode: Mode) -> Emulator {
        let mut emu = Emulator::new();
        emu.cpu.set_cpsr_raw(mode as u32);
        emu.cpu.regs[15] = pc;
        emu
    }

    #[test]
    fn thumb_bx_uses_the_declared_source_register() {
        let mut emu = thumb_emu(0x0300_0000);
        emu.cpu.regs[3] = 0x0800_0101;
        emu.cpu.regs[11] = 0;
        emu.iwram[0] = 0x18;
        emu.iwram[1] = 0x47; // bx r3

        let cycles = emu.step_cpu();

        assert_eq!(cycles, 3);
        assert_eq!(emu.cpu.pc(), 0x0800_0100);
        assert!(emu.cpu.thumb());
    }

    #[test]
    fn thumb_ldr_immediate_word_reads_a_word() {
        let mut emu = thumb_emu(0x0300_0000);
        emu.cpu.regs[4] = 0x0300_0020;
        emu.iwram[0] = 0x63;
        emu.iwram[1] = 0x68; // ldr r3, [r4, #4]
        emu.iwram[0x24..0x28].copy_from_slice(&0x1234_5678u32.to_le_bytes());

        let cycles = emu.step_cpu();

        assert_eq!(cycles, 2);
        assert_eq!(emu.cpu.regs[3], 0x1234_5678);
        assert_eq!(emu.cpu.pc(), 0x0300_0002);
    }

    #[test]
    fn thumb_bl_keeps_the_prefix_for_the_low_halfword() {
        let mut emu = thumb_emu(0x0800_0178);
        let offset = (0x0800_0178 - 0x0800_0000) as usize;
        emu.rom_len = offset + 4;
        emu.rom_staging[offset..offset + 4].copy_from_slice(&[0x13, 0xf0, 0x22, 0xfb]);

        let first_cycles = emu.step_cpu();
        assert_eq!(first_cycles, 1);
        assert_eq!(emu.cpu.pc(), 0x0800_017a);
        assert_eq!(emu.cpu.thumb_bl_prefix, Some(0x0801_317c));

        let second_cycles = emu.step_cpu();
        assert_eq!(second_cycles, 3);
        assert_eq!(emu.cpu.pc(), 0x0801_37c0);
        assert_eq!(emu.cpu.regs[14], 0x0800_017d);
    }

    #[test]
    fn thumb_ldmia_with_base_in_list_keeps_loaded_base_value() {
        let mut emu = thumb_emu(0x0300_0000);
        emu.cpu.regs[3] = 0x0300_0010;
        emu.iwram[0x0000..0x0002].copy_from_slice(&0xcb0au16.to_le_bytes()); // ldmia r3!, {r1, r3}
        emu.iwram[0x0010..0x0014].copy_from_slice(&0x0807_cfccu32.to_le_bytes());
        emu.iwram[0x0014..0x0018].copy_from_slice(&0x0807_cfecu32.to_le_bytes());

        let cycles = emu.step_cpu();

        assert_eq!(cycles, 3);
        assert_eq!(emu.cpu.regs[1], 0x0807_cfcc);
        assert_eq!(emu.cpu.regs[3], 0x0807_cfec);
        assert_eq!(emu.cpu.pc(), 0x0300_0002);
    }

    #[test]
    fn arm_msr_register_updates_the_requested_control_fields() {
        let mut emu = arm_emu(0x0300_0000, Mode::Supervisor);
        emu.cpu.regs[11] = 0x0000_009f; // System mode with IRQ disabled.
        emu.cpu.bank_usr_r13_r14 = [0x0300_7ea8, 0x0800_1234];
        emu.cpu.regs[13] = 0x0300_7fa0;
        emu.cpu.regs[14] = 0x0800_5678;
        emu.iwram[0x0000..0x0004].copy_from_slice(&0xe121_f00bu32.to_le_bytes()); // msr cpsr_c, fp

        let cycles = emu.step_cpu();

        assert_eq!(cycles, 1);
        assert_eq!(emu.cpu.mode(), Mode::System);
        assert!(!emu.cpu.thumb());
        assert_eq!(emu.cpu.pc(), 0x0300_0004);
        assert_eq!(emu.cpu.regs[13], 0x0300_7ea8);
        assert_eq!(emu.cpu.regs[14], 0x0800_1234);
        assert_eq!(emu.cpu.bank_svc_r13_r14, [0x0300_7fa0, 0x0800_5678]);
    }
}
