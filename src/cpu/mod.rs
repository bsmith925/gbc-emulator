// src/cpu/mod.rs

use crate::memory::Memory;

// The CPU registers
#[derive(Clone)]
#[cfg_attr(feature = "savestate", derive(bincode::Encode, bincode::Decode))]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
}

impl Registers {
    pub fn new() -> Self {
        // DMG post-boot register state (what the boot ROM leaves behind). We skip
        // the boot ROM and start at 0x0100, so we must seed these ourselves or
        // games/tests diverge from the very first instruction.
        Self {
            a: 0x01,
            f: 0xB0, // Z=1, N=0, H=1, C=1
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
        }
    }
    // Helper methods to work with paired registers

    pub fn af(&self) -> u16 {
        ((self.a as u16) << 8) | (self.f as u16)
    }

    pub fn set_af(&mut self, value: u16) {
        self.a = (value >> 8) as u8;
        self.f = (value as u8) & 0xF0; // mask: low nibble of F must stay - there's a
        // test_flag_lower_bits_always_zero
    }

    pub fn bc(&self) -> u16 {
        ((self.b as u16) << 8) | (self.c as u16)
    }

    pub fn set_bc(&mut self, value: u16) {
        self.b = (value >> 8) as u8;
        self.c = value as u8;
    }

    pub fn de(&self) -> u16 {
        ((self.d as u16) << 8) | (self.e as u16)
    }

    pub fn set_de(&mut self, value: u16) {
        self.d = (value >> 8) as u8;
        self.e = value as u8;
    }

    pub fn hl(&self) -> u16 {
        ((self.h as u16) << 8) | (self.l as u16)
    }

    pub fn set_hl(&mut self, value: u16) {
        self.h = (value >> 8) as u8;
        self.l = value as u8;
    }

    // Flag manipulation
    pub fn flag_zero(&self) -> bool {
        self.f & 0b1000_0000 != 0
    }

    pub fn set_flag_zero(&mut self, set: bool) {
        if set {
            self.f |= 0b1000_0000;
        } else {
            self.f &= 0b0111_1111;
        }
    }

    pub fn flag_subtract(&self) -> bool {
        self.f & 0b0100_0000 != 0
    }

    pub fn set_flag_subtract(&mut self, set: bool) {
        if set {
            self.f |= 0b0100_0000;
        } else {
            self.f &= 0b1011_1111; // clear bit 6 (N), not bit 5
        }
    }

    pub fn flag_half_carry(&self) -> bool {
        self.f & 0b0010_0000 != 0
    }

    pub fn set_flag_half_carry(&mut self, set: bool) {
        if set {
            self.f |= 0b0010_0000;
        } else {
            self.f &= 0b1101_1111;
        }
    }

    pub fn flag_carry(&self) -> bool {
        self.f & 0b0001_0000 != 0
    }

    pub fn set_flag_carry(&mut self, set: bool) {
        if set {
            self.f |= 0b0001_0000;
        } else {
            self.f &= 0b1110_1111
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

impl Register {
    /// Get the value of this register from a CPU
    pub fn get(&self, cpu: &Cpu) -> u8 {
        match self {
            Register::A => cpu.registers.a,
            Register::B => cpu.registers.b,
            Register::C => cpu.registers.c,
            Register::D => cpu.registers.d,
            Register::E => cpu.registers.e,
            Register::H => cpu.registers.h,
            Register::L => cpu.registers.l,
        }
    }

    /// Set the value of this register in a CPU
    pub fn set(&self, cpu: &mut Cpu, value: u8) {
        match self {
            Register::A => cpu.registers.a = value,
            Register::B => cpu.registers.b = value,
            Register::C => cpu.registers.c = value,
            Register::D => cpu.registers.d = value,
            Register::E => cpu.registers.e = value,
            Register::H => cpu.registers.h = value,
            Register::L => cpu.registers.l = value,
        }
    }

    /// Get the name of this register as a string
    pub fn name(&self) -> &'static str {
        match self {
            Register::A => "A",
            Register::B => "B",
            Register::C => "C",
            Register::D => "D",
            Register::E => "E",
            Register::H => "H",
            Register::L => "L",
        }
    }

    /// Get all registers as an array (useful for iteration)
    pub fn all() -> [Register; 7] {
        [
            Register::B,
            Register::C,
            Register::D,
            Register::E,
            Register::H,
            Register::L,
            Register::A,
        ]
    }
}

#[derive(Clone)]
#[cfg_attr(feature = "savestate", derive(bincode::Encode, bincode::Decode))]
pub struct Cpu {
    pub registers: Registers,
    pub pc: u16, // Program Counter
    pub sp: u16, // Stack Pointer
    pub memory: Memory,
    pub cycles: u64, // Total cycles executed
    pub halted: bool,
    pub interrupts_enabled: bool,
    ime_pending: bool, // EI armed IME to turn on after the next instruction
    halt_bug: bool,    // HALT bug: the next opcode fetch must not advance PC
    pub div_counter: u16,  // accumulates cycles for DIV (DIV = its high byte)
    pub tima_counter: u32, // accumulates cycles for TIMA
    pub ppu_dots: u32,     // accumulates cycles within the current scanline
    pub framebuffer: [(u8, u8, u8); 160 * 144], // RGB per pixel
    // Per-scanline BG info for sprite priority: the BG/window color id (0-3) and
    // whether that pixel had CGB BG-priority. Filled by render_scanline, read by
    // render_sprites.
    bg_color: [u8; 160],
    bg_prio: [bool; 160],
    pub trace: bool,       // Gameboy Doctor trace mode (GBC_TRACE env var)
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            registers: Registers::new(),
            pc: 0x0100, // Game start at 0x100 (after boot ROM)
            sp: 0xFFFE, // Stack starts at top of memory
            memory: Memory::new(),
            cycles: 0,
            halted: false,
            interrupts_enabled: false,
            ime_pending: false,
            halt_bug: false,
            div_counter: 0,
            tima_counter: 0,
            ppu_dots: 0,
            framebuffer: [(224, 248, 208); 160 * 144], // DMG blank = lightest green
            bg_color: [0; 160],
            bg_prio: [false; 160],
            trace: std::env::var("GBC_TRACE").is_ok(),
        }
    }

    /// Execute one instruction
    pub fn step(&mut self) {
        // Check for interrupts first - this can dispatch AND wake us from HALT.
        self.handle_interrupts();

        if self.halted {
            // When halted, just increment cycles
            self.cycles += 4;
            self.step_timer(4);
            // PPU/APU run at base rate; in double-speed they get half the cycles.
            let ppu_cycles = 4 >> (self.memory.double_speed as u32);
            self.step_ppu(ppu_cycles);
            self.memory.apu.step(ppu_cycles);
            return;
        }

        // Gameboy Doctor trace: log state BEFORE the instruction, not while halted.
        if self.trace {
            println!(
                "A:{:02X} F:{:02X} B:{:02X} C:{:02X} D:{:02X} E:{:02X} H:{:02X} L:{:02X} \
                 SP:{:04X} PC:{:04X} PCMEM:{:02X},{:02X},{:02X},{:02X}",
                self.registers.a,
                self.registers.f,
                self.registers.b,
                self.registers.c,
                self.registers.d,
                self.registers.e,
                self.registers.h,
                self.registers.l,
                self.sp,
                self.pc,
                self.memory.read_byte(self.pc),
                self.memory.read_byte(self.pc.wrapping_add(1)),
                self.memory.read_byte(self.pc.wrapping_add(2)),
                self.memory.read_byte(self.pc.wrapping_add(3)),
            );
        }

        // EI enables interrupts AFTER the next instruction: if a previous EI
        // armed it, this is that next instruction, so enable IME once it's done.
        let enable_ime = self.ime_pending;

        let before = self.cycles;
        // Fetch the opcode
        let opcode = self.fetch_byte();

        // Execute the instruction
        self.execute(opcode);

        if enable_ime {
            self.interrupts_enabled = true;
            self.ime_pending = false;
        }

        let elapsed = (self.cycles - before) as u32;
        self.step_timer(elapsed); // timer follows the CPU (2x in double-speed)
        // PPU/APU run at base rate; halve the cycle feed in double-speed.
        let ppu_cycles = elapsed >> (self.memory.double_speed as u32);
        self.step_ppu(ppu_cycles);
        self.memory.apu.step(ppu_cycles);
    }

    /// Advance the timer by the number of cycles the last instruction took.
    fn step_timer(&mut self, cycles: u32) {
        // DIV: free-running. The reigster is the HIGH byte of the 16-bit counter,
        // so it increments once every 256 cycles (16384 Hz)
        self.div_counter = self.div_counter.wrapping_add(cycles as u16);
        self.memory
            .write_byte(0xFF04, (self.div_counter >> 8) as u8);

        // TIMA only runs when TAX bit 2 is set
        let tac = self.memory.read_byte(0xFF07);
        if tac & 0x04 == 0 {
            return; // time disabled
        }

        // bits 1-0 pick how many CPU cycles per TIMA tick
        let period: u32 = match tac & 0x03 {
            0 => 1024, // 4096 Hz
            1 => 16,   // 262144 Hz
            2 => 64,   // 65536 Hz
            3 => 256,  // 16384 Hz
            _ => unreachable!(),
        };

        self.tima_counter += cycles;
        while self.tima_counter >= period {
            self.tima_counter -= period;
            let tima = self.memory.read_byte(0xFF05);
            if tima == 0xFF {
                // overflow: reload from TMA and request the Timer interrupt (bit 2)
                let tma = self.memory.read_byte(0xFF06);
                self.memory.write_byte(0xFF05, tma);
                self.request_interrupt(2);
            } else {
                self.memory.write_byte(0xFF05, tima + 1);
            }
        }
    }

    /// Advance the PPU by the cycles the last instruction took
    fn step_ppu(&mut self, cycles: u32) {
        let lcdc = self.memory.read_byte(0xFF40);
        if lcdc & 0x80 == 0 {
            // LCD off: reset to lie 0, no timing, no draw
            self.ppu_dots = 0;
            self.memory.write_byte(0xFF44, 0);
            return;
        }

        // 1. accumulate dots for the current scanline
        // add `cycles` to self.ppu_dots
        self.ppu_dots += cycles;

        // 2. each time a full scanline (456 dots) has elapsed ...
        while self.ppu_dots >= 456 {
            self.ppu_dots -= 456;

            // 3. advance to the next scanline (wrap 153 -> 0)
            let ly = self.memory.read_byte(0xFF44);
            let next_ly = if ly >= 153 { 0 } else { ly + 1 };
            self.memory.write_byte(0xFF44, next_ly);

            // 4. start of VBlank -> request the VBlank interrupt
            if next_ly == 144 {
                self.request_interrupt(0);
            }

            // 5. Render the line that is now STARTING, using registers as they
            // stand at its start (i.e. after the previous line's HBlank handler).
            // This is what makes per-line raster effects (split-screen status
            // bars, parallax via HBlank SCX changes) land on the correct line.
            if next_ly < 144 {
                self.render_scanline(next_ly);
                self.render_sprites(next_ly);
            }
        }
        self.update_stat();
    }

    /// Recompute STAT's status bits (mode + LYC coincidence) from LY + dot position
    fn update_stat(&mut self) {
        let ly = self.memory.read_byte(0xFF44);

        // current mode
        let mode: u8 = if ly >= 144 {
            1
        } else if self.ppu_dots < 80 {
            2
        } else if self.ppu_dots < 252 {
            3
        } else {
            0
        };

        // LYC=LY coincidence
        let lyc = self.memory.read_byte(0xFF45);
        let coincidence = ly == lyc; // true when ly equals lyc

        // write bits 0-2, PRESERVING bits 3-7 (the game's enable bits)
        let old_stat = self.memory.read_byte(0xFF41);
        let new_stat = (old_stat & !7) | mode | (coincidence as u8) << 2;
        self.memory.write_byte(0xFF41, new_stat);

        // STAT interrupt (IF bit 1), fired on rising edges
        let prev_mode = old_stat & 0x03;
        let prev_coincidence = old_stat & 0x04 != 0;

        let mut fire = false;

        // mode just changed -> check the enable bit for the mode we ENTERED
        if mode != prev_mode {
            let enable = match mode {
                0 => 0x08, // HBlank
                2 => 0x20, // OAM
                1 => 0x10, // VBlank
                _ => 0,    // mode 3: no STAT source
            };
            if old_stat & enable != 0 {
                fire = true;
            }
        }

        if coincidence && !prev_coincidence && (old_stat & 0x40 != 0) {
            fire = true;
        }

        if fire {
            self.request_interrupt(1);
        }
    }

    /// Look up one pixel's color id (0-3) from a tile map. `px_x`/`px_y` are
    /// coordinates in the 256x256 map space; `map_base` is 0x9800 or 0x9C00.
    /// Tile-data addressing follows LCDC bit 4 (unsigned 0x8000 vs signed 0x9000).
    fn tile_pixel(&self, map_base: u16, px_x: u16, px_y: u16, lcdc: u8) -> u8 {
        let tile_col = (px_x / 8) & 31;
        let tile_row = (px_y / 8) & 31;
        let tile_id = self.memory.read_byte(map_base + tile_row * 32 + tile_col);

        let tile_addr: u16 = if lcdc & 0x10 != 0 {
            0x8000 + (tile_id as u16) * 16
        } else {
            (0x9000_i32 + (tile_id as i8 as i32) * 16) as u16
        };

        let row_addr = tile_addr + (px_y % 8) * 2;
        let low = self.memory.read_byte(row_addr);
        let high = self.memory.read_byte(row_addr + 1);
        decode_tile_row(low, high)[(px_x % 8) as usize]
    }

    /// CGB version: returns (rgb, color_id, bg_priority) for a BG/window pixel,
    /// honoring the per-tile attribute byte in VRAM bank 1 (palette, tile bank,
    /// X/Y flip, priority). color_id and priority feed sprite priority.
    fn cgb_tile_pixel(&self, map_base: u16, px_x: u16, px_y: u16, lcdc: u8) -> ((u8, u8, u8), u8, bool) {
        let tile_col = (px_x / 8) & 31;
        let tile_row = (px_y / 8) & 31;
        let map_addr = map_base + tile_row * 32 + tile_col;

        let tile_id = self.memory.vram_read(0, map_addr); // tile index: bank 0
        let attr = self.memory.vram_read(1, map_addr); // attributes: bank 1
        let palette = (attr & 0x07) as usize;
        let tile_bank = ((attr >> 3) & 1) as usize;
        let x_flip = attr & 0x20 != 0;
        let y_flip = attr & 0x40 != 0;
        let priority = attr & 0x80 != 0;

        let tile_addr: u16 = if lcdc & 0x10 != 0 {
            0x8000 + (tile_id as u16) * 16
        } else {
            (0x9000_i32 + (tile_id as i8 as i32) * 16) as u16
        };

        let mut row = px_y % 8;
        if y_flip {
            row = 7 - row;
        }
        let row_addr = tile_addr + row * 2;
        let low = self.memory.vram_read(tile_bank, row_addr);
        let high = self.memory.vram_read(tile_bank, row_addr + 1);

        let mut col = (px_x % 8) as usize;
        if x_flip {
            col = 7 - col;
        }
        let color = decode_tile_row(low, high)[col];
        (self.memory.cgb_bg_color(palette, color as usize), color, priority)
    }

    /// Render one scanline (background + window) into the framebuffer.
    fn render_scanline(&mut self, ly: u8) {
        let lcdc = self.memory.read_byte(0xFF40);
        let bgp = self.memory.read_byte(0xFF47);
        let cgb = self.memory.is_cgb();

        // DMG only: BG disabled (LCDC bit 0) blanks the line. In CGB that bit
        // means something else and the background is always drawn.
        if !cgb && lcdc & 0x01 == 0 {
            for x in 0..160usize {
                self.framebuffer[ly as usize * 160 + x] = dmg_rgb(0);
                self.bg_color[x] = 0;
                self.bg_prio[x] = false;
            }
            return;
        }

        let scy = self.memory.read_byte(0xFF42);
        let scx = self.memory.read_byte(0xFF43);
        let wy = self.memory.read_byte(0xFF4A);
        let wx = self.memory.read_byte(0xFF4B);

        let window_on = lcdc & 0x20 != 0; // LCDC bit 5
        let bg_map = if lcdc & 0x08 != 0 { 0x9C00 } else { 0x9800 }; // bit 3
        let win_map = if lcdc & 0x40 != 0 { 0x9C00 } else { 0x9800 }; // bit 6

        for x in 0..160u16 {
            // The window covers a pixel when enabled and we're past its origin
            // (top-left at screen (WX-7, WY)). The window does not scroll.
            let in_window = window_on && ly as u16 >= wy as u16 && x + 7 >= wx as u16;

            // Pick the map + coordinates for this pixel (window or scrolled BG).
            let (map, mx, my) = if in_window {
                (win_map, x + 7 - wx as u16, ly as u16 - wy as u16)
            } else {
                (
                    bg_map,
                    (x + scx as u16) & 0xFF,
                    (ly as u16 + scy as u16) & 0xFF,
                )
            };

            let (rgb, color_id, prio) = if cgb {
                self.cgb_tile_pixel(map, mx, my, lcdc)
            } else {
                let c = self.tile_pixel(map, mx, my, lcdc);
                (dmg_rgb(apply_palette(bgp, c)), c, false)
            };
            self.bg_color[x as usize] = color_id;
            self.bg_prio[x as usize] = prio;
            self.framebuffer[ly as usize * 160 + x as usize] = rgb;
        }
    }

    /// Draw sprites (OAM objects) that overlap scanline `ly`, on top of BG/window.
    fn render_sprites(&mut self, ly: u8) {
        let lcdc = self.memory.read_byte(0xFF40);
        if lcdc & 0x02 == 0 {
            return; // LCDC bit 1: sprites (OBJ) disabled
        }
        let height: i16 = if lcdc & 0x04 != 0 { 16 } else { 8 }; // bit 2: 8x16 vs 8x8
        let cgb = self.memory.is_cgb();

        // Collect the first 10 sprites (in OAM order) that overlap this line.
        // Real hardware draws at most 10 per line - the cause of sprite flicker.
        let mut line: Vec<u16> = Vec::with_capacity(10);
        for i in 0..40u16 {
            let top = self.memory.read_byte(0xFE00 + i * 4) as i16 - 16;
            if (ly as i16) >= top && (ly as i16) < top + height {
                line.push(i);
                if line.len() == 10 {
                    break;
                }
            }
        }

        // Draw back-to-front so the highest-priority sprite lands last (on top).
        // CGB: lower OAM index wins. DMG: smaller X wins, ties by lower index.
        line.sort_by(|&a, &b| {
            if cgb {
                b.cmp(&a)
            } else {
                let ax = self.memory.read_byte(0xFE00 + a * 4 + 1);
                let bx = self.memory.read_byte(0xFE00 + b * 4 + 1);
                bx.cmp(&ax).then(b.cmp(&a))
            }
        });

        for &i in &line {
            let base = 0xFE00 + i * 4;
            let oam_y = self.memory.read_byte(base) as i16; // screen Y + 16
            let oam_x = self.memory.read_byte(base + 1) as i16; // screen X + 8
            let tile = self.memory.read_byte(base + 2);
            let flags = self.memory.read_byte(base + 3);

            let top = oam_y - 16;
            let left = oam_x - 8;

            // Does this scanline fall within the sprite's vertical span?
            let mut row = ly as i16 - top;
            if row < 0 || row >= height {
                continue;
            }
            if flags & 0x40 != 0 {
                row = height - 1 - row; // Y-flip
            }

            // 8x16: low bit of the tile index is ignored; rows 0-7 use the first
            // tile, rows 8-15 the next.
            let tile_index = if height == 16 {
                (tile & 0xFE) + (row / 8) as u8
            } else {
                tile
            };
            let row_in_tile = (row % 8) as u16;

            // Sprites always use unsigned 0x8000 tile-data addressing. In CGB
            // mode flag bit 3 picks which VRAM bank holds the tile.
            let tile_bank = if cgb { ((flags >> 3) & 1) as usize } else { 0 };
            let addr = 0x8000 + tile_index as u16 * 16 + row_in_tile * 2;
            let low = self.memory.vram_read(tile_bank, addr);
            let high = self.memory.vram_read(tile_bank, addr + 1);
            let pixels = decode_tile_row(low, high);

            for col in 0..8i16 {
                let px = if flags & 0x20 != 0 { 7 - col } else { col }; // X-flip
                let color = pixels[px as usize];
                if color == 0 {
                    continue; // color 0 is transparent for sprites
                }
                let screen_x = left + col;
                if !(0..160).contains(&screen_x) {
                    continue; // off-screen horizontally
                }

                // Priority: the BG covers this sprite pixel when the BG pixel is
                // opaque (color 1-3) AND either this sprite is flagged "behind BG"
                // or the BG tile has CGB priority. In CGB, LCDC bit 0 clear
                // disables BG priority (sprites always win).
                let bg_c = self.bg_color[screen_x as usize];
                let bg_wins = bg_c != 0
                    && (flags & 0x80 != 0 || self.bg_prio[screen_x as usize])
                    && (!cgb || lcdc & 0x01 != 0);
                if bg_wins {
                    continue;
                }

                let rgb = if cgb {
                    // CGB: flag bits 0-2 select one of 8 OBJ palettes.
                    self.memory.cgb_obj_color((flags & 0x07) as usize, color as usize)
                } else {
                    // DMG: flag bit 4 picks OBP0/OBP1.
                    let dmg_pal = if flags & 0x10 != 0 {
                        self.memory.read_byte(0xFF49)
                    } else {
                        self.memory.read_byte(0xFF48)
                    };
                    dmg_rgb(apply_palette(dmg_pal, color))
                };
                self.framebuffer[ly as usize * 160 + screen_x as usize] = rgb;
            }
        }
    }

    /// Request an interrupt by setting its IF (0xFF0F) bit. bit: 0=VBlank .. 4=Joypad
    pub fn request_interrupt(&mut self, bit: u8) {
        let iflag = self.memory.read_byte(0xFF0F);
        self.memory.write_byte(0xFF0F, iflag | (1 << bit));
    }

    /// A human-readable one-line snapshot of CPU + key hardware state.
    /// Returns the string (caller decides whether to print/log/assert).
    pub fn debug_state(&self) -> String {
        // Decode F into letters: the flag's letter if set, '-' if clear.
        let flags = format!(
            "{}{}{}{}",
            if self.registers.flag_zero() { 'Z' } else { '-' },
            if self.registers.flag_subtract() { 'N' } else { '-' },
            if self.registers.flag_half_carry() { 'H' } else { '-' },
            if self.registers.flag_carry() { 'C' } else { '-' },
        );

        // Occupancy: split VRAM so we can see WHERE content is.
        let tiledata_nz = (0x8000u16..0x9800)
            .filter(|&a| self.memory.read_byte(a) != 0)
            .count();
        let map1_nz = (0x9800u16..0x9C00) // BG map (or window if LCDC bit6=0)
            .filter(|&a| self.memory.read_byte(a) != 0)
            .count();
        let map2_nz = (0x9C00u16..0xA000) // window map (or BG if LCDC bit3=1)
            .filter(|&a| self.memory.read_byte(a) != 0)
            .count();
        let fb_nz = self.framebuffer.iter().filter(|&&p| p != dmg_rgb(0)).count();

        format!(
            "A:{:02X} F:{:02X}({}) B:{:02X} C:{:02X} D:{:02X} E:{:02X} H:{:02X} L:{:02X} \
             SP:{:04X} PC:{:04X} | LCDC:{:02X} STAT:{:02X} LY:{:02X} IE:{:02X} IF:{:02X} IME:{} \
             | SCY:{:02X} SCX:{:02X} BGP:{:02X} WY:{:02X} WX:{:02X} \
             | tiledata_nz:{} map@9800_nz:{} map@9C00_nz:{} fb_nz:{}",
            self.registers.a,
            self.registers.f,
            flags,
            self.registers.b,
            self.registers.c,
            self.registers.d,
            self.registers.e,
            self.registers.h,
            self.registers.l,
            self.sp,
            self.pc,
            self.memory.read_byte(0xFF40),
            self.memory.read_byte(0xFF41),
            self.memory.read_byte(0xFF44),
            self.memory.read_byte(0xFFFF),
            self.memory.read_byte(0xFF0F),
            self.interrupts_enabled,
            self.memory.read_byte(0xFF42),
            self.memory.read_byte(0xFF43),
            self.memory.read_byte(0xFF47),
            self.memory.read_byte(0xFF4A),
            self.memory.read_byte(0xFF4B),
            tiledata_nz,
            map1_nz,
            map2_nz,
            fb_nz,
        )
    }

    /// Check for and dispatch a pending interrupt. Runs before each fetch.
    fn handle_interrupts(&mut self) {
        let ie = self.memory.read_byte(0xFFFF); // which interrupts are enabled
        let iflag = self.memory.read_byte(0xFF0F); // which are requested
        let pending = ie & iflag & 0x1F; // enabled AND requested, low 5 bits

        if pending == 0 {
            return; // nothing to do
        }

        // A pending interrupt wakes the CPU from HALT - even if IME is off.
        self.halted = false;

        if !self.interrupts_enabled {
            return; // IME off: woke from HALT but no NOT dispatch
        }
        // Highest priority = lowest set bit (VBlank=0 wins over Joypad=4)
        let bit = pending.trailing_zeros() as u8;

        // Acknowledge: clear this requeset, turn off IME for the handler.
        self.memory.write_byte(0xFF0F, iflag & !(1 << bit));
        self.interrupts_enabled = false;

        // Hardware CALL: push pc, jump to the vector.
        self.sp = self.sp.wrapping_sub(2);
        self.memory.write_word(self.sp, self.pc);
        self.pc = 0x0040 + (bit as u16) * 8;
        self.cycles += 20;
    }

    /// Fetch a byte from PC and increment PC
    fn fetch_byte(&mut self) -> u8 {
        let byte = self.memory.read_byte(self.pc);
        if self.halt_bug {
            // HALT bug: this fetch reads the byte but PC does not advance, so the
            // next fetch reads it again (the byte executes twice).
            self.halt_bug = false;
        } else {
            self.pc = self.pc.wrapping_add(1);
        }
        byte
    }

    /// Fetch a 16-bit word from PC and increment PC twice
    fn fetch_word(&mut self) -> u16 {
        let word = self.memory.read_word(self.pc);
        self.pc = self.pc.wrapping_add(2);
        word
    }

    /// Execute an instruction based on opcode
    fn execute(&mut self, opcode: u8) {
        match opcode {
            // LD rr, d16
            0x01 | 0x11 | 0x21 | 0x31 => {
                let idx = (opcode >> 4) & 0x03; // bits 5-4 which pair
                let value = self.fetch_word(); // read the 16-bit immediate (little endian)
                self.write_rr(idx, false, value); // rp table -> BC/DE/HL/SP
                self.cycles += 12;
            }

            // LD (rr), A - store A into memory at BC / DE / HL+  HL-
            0x02 | 0x12 | 0x22 | 0x32 => {
                let addr = match (opcode >> 4) & 0x03 {
                    0 => self.registers.bc(),
                    1 => self.registers.de(),
                    2 => {
                        let a = self.registers.hl();
                        self.registers.set_hl(a.wrapping_add(1));
                        a
                    }
                    3 => {
                        let a = self.registers.hl();
                        self.registers.set_hl(a.wrapping_sub(1));
                        a
                    }
                    _ => unreachable!(),
                };
                self.memory.write_byte(addr, self.registers.a);
                self.cycles += 8;
            }

            // INC rr
            0x03 | 0x13 | 0x23 | 0x33 => {
                let idx = (opcode >> 4) & 0x03;
                let value = self.read_rr(idx, false).wrapping_add(1);
                self.write_rr(idx, false, value);
                self.cycles += 8;
            }

            // ADD HL, rr
            0x09 | 0x19 | 0x29 | 0x39 => {
                let idx = (opcode >> 4) & 0x03;
                let hl = self.registers.hl();
                let rr = self.read_rr(idx, false);
                let result = hl.wrapping_add(rr);

                // Z is intentionally left alone
                self.registers.set_flag_subtract(false);
                self.registers
                    .set_flag_half_carry((hl & 0x0FFF) + (rr & 0x0FFF) > 0x0FFF);
                self.registers
                    .set_flag_carry((hl as u32) + (rr as u32) > 0xFFFF);

                self.registers.set_hl(result);
                self.cycles += 8;
            }

            // LD A, (rr) - load A from memory at BC / DE / HL+ / HL-
            0x0A | 0x1A | 0x2A | 0x3A => {
                let addr = match (opcode >> 4) & 0x03 {
                    0 => self.registers.bc(),
                    1 => self.registers.de(),
                    2 => {
                        let a = self.registers.hl();
                        self.registers.set_hl(a.wrapping_add(1));
                        a
                    }
                    3 => {
                        let a = self.registers.hl();
                        self.registers.set_hl(a.wrapping_sub(1));
                        a
                    }
                    _ => unreachable!(),
                };
                self.registers.a = self.memory.read_byte(addr); // read INTO a
                self.cycles += 8;
            }

            // DEC rr
            0x0B | 0x1B | 0x2B | 0x3B => {
                let idx = (opcode >> 4) & 0x03;
                let value = self.read_rr(idx, false).wrapping_sub(1);
                self.write_rr(idx, false, value);
                self.cycles += 8;
            }

            // NOP - No Operation
            0x00 => {
                self.cycles += 4;
            }

            // INC B - Increment B
            0x04 => {
                self.registers.b = self.alu_inc(self.registers.b);
                self.cycles += 4;
            }

            // DEC B - Decrement B
            0x05 => {
                self.registers.b = self.alu_dec(self.registers.b);
                self.cycles += 4;
            }

            // LD B, d8 - Load 8-bit immediate into B
            0x06 => {
                self.registers.b = self.fetch_byte();
                self.cycles += 8;
            }

            // INC C - Increment C
            0x0C => {
                self.registers.c = self.alu_inc(self.registers.c);
                self.cycles += 4;
            }

            // DEC C - Decrement C
            0x0D => {
                self.registers.c = self.alu_dec(self.registers.c);
                self.cycles += 4;
            }

            // LD C, d8
            0x0E => {
                self.registers.c = self.fetch_byte();
                self.cycles += 8;
            }

            // INC D - Increment D
            0x14 => {
                self.registers.d = self.alu_inc(self.registers.d);
                self.cycles += 4;
            }

            // DEC D - Decrement D
            0x15 => {
                self.registers.d = self.alu_dec(self.registers.d);
                self.cycles += 4;
            }

            // LD D, d8
            0x16 => {
                self.registers.d = self.fetch_byte();
                self.cycles += 8;
            }

            // JR e8 - unconditional relative jump
            0x18 => {
                // three-cast offset math, pc += offset, 12 cycles
                let offset = self.fetch_byte() as i8; // signed -1238..127; advances pc by 1
                // let pc = self.pc as i16; // widen pc so we can add
                // self.pc = (pc + offset as i16) as u16; // add, cast back to u16
                self.pc = self.pc.wrapping_add(offset as i16 as u16); // sign-extend,
                // reinterpret, modular add
                self.cycles += 12;
            }

            // INC E - Increment E
            0x1C => {
                self.registers.e = self.alu_inc(self.registers.e);
                self.cycles += 4;
            }

            // DEC E - Decrement E
            0x1D => {
                self.registers.e = self.alu_dec(self.registers.e);
                self.cycles += 4;
            }

            // LD E, d8
            0x1E => {
                self.registers.e = self.fetch_byte();
                self.cycles += 8;
            }

            // JR cc, e8 - conditional relative jump
            0x20 | 0x28 | 0x30 | 0x38 => {
                // extract cc, fetch offset ALWAYS, then check_condition
                let cc = (opcode >> 3) & 0x03;
                let offset = self.fetch_byte() as i8; // ALWAYS fetch (advances pc past operand)
                if self.check_condition(cc) {
                    // let pc = self.pc as i16;
                    // self.pc = (pc + offset as i16) as u16;
                    self.pc = self.pc.wrapping_add(offset as i16 as u16);
                    self.cycles += 12; //taken
                } else {
                    self.cycles += 8; // not taken
                }
            }

            // INC H - Increment H
            0x24 => {
                self.registers.h = self.alu_inc(self.registers.h);
                self.cycles += 4;
            }

            // DEC H - Decrement H
            0x25 => {
                self.registers.h = self.alu_dec(self.registers.h);
                self.cycles += 4;
            }

            // LD H, d8
            0x26 => {
                self.registers.h = self.fetch_byte();
                self.cycles += 8;
            }

            // INC L - Increment L
            0x2C => {
                self.registers.l = self.alu_inc(self.registers.l);
                self.cycles += 4;
            }

            // DEC L - Decrement L
            0x2D => {
                self.registers.l = self.alu_dec(self.registers.l);
                self.cycles += 4;
            }

            // LD L, d8
            0x2E => {
                self.registers.l = self.fetch_byte();
                self.cycles += 8;
            }

            // INC A - Increment A
            0x3C => {
                self.registers.a = self.alu_inc(self.registers.a);
                self.cycles += 4;
            }

            // DEC A - Decrement A
            0x3D => {
                self.registers.a = self.alu_dec(self.registers.a);
                self.cycles += 4;
            }

            0x3E => {
                self.registers.a = self.fetch_byte();
                self.cycles += 8;
            }

            // LD r, r' - register/(HL) to register/(HL); 0x76 HALT
            0x40..=0x7F => {
                if opcode == 0x76 {
                    // HALT bug: if IME is off but an interrupt is already pending,
                    // the CPU doesn't halt - instead the next byte is read twice.
                    let ie = self.memory.read_byte(0xFFFF);
                    let iflag = self.memory.read_byte(0xFF0F);
                    if !self.interrupts_enabled && (ie & iflag & 0x1F) != 0 {
                        self.halt_bug = true;
                    } else {
                        self.halted = true;
                    }
                    self.cycles += 4;
                } else {
                    let dest = (opcode >> 3) & 0x07; // bits 5-3
                    let src = opcode & 0x07; // bits 2-0
                    let value = self.read_operand(src);
                    self.write_operand(dest, value);
                    self.cycles += if src == 6 || dest == 6 { 8 } else { 4 };
                }
            }

            // ALU ops: ADD/ADC/SUB/SBC/AND/XOR/OR/CP A, r (r = B,C,D,E,H,L,(HL),A)
            0x80..=0xBF => {
                let src = opcode & 0x07; // bitz 2-0: which operand
                let op = (opcode >> 3) & 0x07; // bits 5-3: which operation
                let value = self.read_operand(src);

                match op {
                    0 => self.alu_add(value),
                    1 => self.alu_adc(value),
                    2 => self.alu_sub(value),
                    3 => self.alu_sbc(value),
                    4 => self.alu_and(value),
                    5 => self.alu_xor(value),
                    6 => self.alu_or(value),
                    7 => self.alu_cp(value),
                    _ => unreachable!(),
                }
                // (HL) costs 8 cycles, all others 4
                self.cycles += if src == 6 { 8 } else { 4 };
            }

            // JP a16 - unconditional jump
            0xC3 => {
                // fetch the 16-bit target, set pc to it, 16 cycles
                let target = self.fetch_word();
                self.pc = target;
                self.cycles += 16;
            }

            // JP cc, a16 - conditional jump
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                // 1. extract cc with (opcode >> 3) & 0x03
                let cc = (opcode >> 3) & 0x03;
                // 2. fetch the target ALWAYS (advances pc past the operand)
                let target = self.fetch_word();
                // 3. if check_condition(cc): set pc, 16 cycles
                //  else: 12 cycles
                if self.check_condition(cc) {
                    self.pc = target;
                    self.cycles += 16;
                } else {
                    self.cycles += 12;
                }
            }

            // PUSH rr - decrement SP by 2, write the pair to the stack
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                let idx = (opcode >> 4) & 0x03; // bits 5-4 -> BC/DE/HL/AF
                let value = self.read_rr(idx, true); // use_af=true: index 3 == AF
                self.sp = self.sp.wrapping_sub(2); // grow the stack downward FIRSZT
                self.memory.write_word(self.sp, value); // little-endian: low byte at sp,
                // high at sp +1
                self.cycles += 16;
            }

            // RET cc - conditional return
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                // (20 taken / 8 not)
                let cc = (opcode >> 3) & 0x03;
                if self.check_condition(cc) {
                    let addr = self.memory.read_word(self.sp);
                    self.sp = self.sp.wrapping_add(2);
                    self.pc = addr;
                    self.cycles += 20;
                } else {
                    self.cycles += 8;
                }
            }

            // POP rr - read the pair from the stack, increment SP by 2
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let idx = (opcode >> 4) & 0x03;
                let value = self.memory.read_word(self.sp); // read BEFORE moving sp
                self.write_rr(idx, true, value); // use_af=true: index 3 == AF
                self.sp = self.sp.wrapping_add(2); // reclaim the space
                self.cycles += 12;
            }

            // CALL cc, a16 - conditional call
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let cc = (opcode >> 3) & 0x03;
                let target = self.fetch_word(); // ALWAYS fetch (clears the operand)
                if self.check_condition(cc) {
                    self.sp = self.sp.wrapping_sub(2);
                    self.memory.write_word(self.sp, self.pc); // push return addr
                    self.pc = target;
                    self.cycles += 24; //taken
                } else {
                    self.cycles += 12; // not taken
                }
            }

            // RST t - call a fixed vector (t*8)
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                let target = ((opcode >> 3) & 0x07) as u16 * 8; // bitc 5-3 -> vector address
                self.sp = self.sp.wrapping_sub(2);
                self.memory.write_word(self.sp, self.pc); // push return address
                self.pc = target;
                self.cycles += 16;
            }

            // RET - pop return address into pc
            0xC9 => {
                let addr = self.memory.read_word(self.sp); // read saved return
                // address
                self.sp = self.sp.wrapping_add(2); // reclaim stack space
                self.pc = addr;
                self.cycles += 16;
            }

            // CB prefix - fetch the second  opcode byte and decode the extended table
            0xCB => {
                let cb_opcode = self.fetch_byte();
                self.execute_cb(cb_opcode);
            }

            // CALL a16 - push return address, jump to target
            0xCD => {
                let target = self.fetch_word(); // pc now points at the NEXT instruction
                // (return addr)
                self.sp = self.sp.wrapping_sub(2); // make room on the stack
                self.memory.write_word(self.sp, self.pc); // push the return address
                self.pc = target;
                self.cycles += 24;
            }

            // RETI - return and enable interrupts
            0xD9 => {
                let addr = self.memory.read_word(self.sp);
                self.sp = self.sp.wrapping_add(2);

                self.pc = addr;
                self.interrupts_enabled = true; // RETI enables IME immediately
                self.ime_pending = false;
                self.cycles += 16;
            }

            // JP (HL) - jump to address in HL (NOT a memory read)
            0xE9 => {
                //pc = HL, 4 cycles
                self.pc = self.registers.hl();
                self.cycles += 4;
            }

            // DI - disable interrupts (clear IME immediately, cancel any pending EI)
            0xF3 => {
                self.interrupts_enabled = false;
                self.ime_pending = false;
                self.cycles += 4;
            }

            // EI - enable interrupts, but not until AFTER the next instruction.
            0xFB => {
                self.ime_pending = true;
                self.cycles += 4;
            }

            // LDH (a8), A — write A to high memory 0xFF00+n
            0xE0 => {
                let n = self.fetch_byte();
                let addr = 0xFF00 + n as u16;
                self.memory.write_byte(addr, self.registers.a);
                self.cycles += 12;
            }

            // LDH A, (a8) — read high memory 0xFF00+n into A
            0xF0 => {
                let n = self.fetch_byte();
                let addr = 0xFF00 + n as u16;
                self.registers.a = self.memory.read_byte(addr);
                self.cycles += 12;
            }

            // LD (C), A — write A to high memory 0xFF00+C
            0xE2 => {
                let addr = 0xFF00 + self.registers.c as u16;
                self.memory.write_byte(addr, self.registers.a);
                self.cycles += 8;
            }

            // LD A, (C) — read high memory 0xFF00+C into A
            0xF2 => {
                let addr = 0xFF00 + self.registers.c as u16;
                self.registers.a = self.memory.read_byte(addr);
                self.cycles += 8;
            }

            // LD (a16), A — write A to absolute 16-bit address
            0xEA => {
                let addr = self.fetch_word();
                self.memory.write_byte(addr, self.registers.a);
                self.cycles += 16;
            }

            // LD A, (a16) — read absolute 16-bit address into A
            0xFA => {
                let addr = self.fetch_word();
                self.registers.a = self.memory.read_byte(addr);
                self.cycles += 16;
            }

            // ADD A, d8 — A = A + n
            0xC6 => {
                let n = self.fetch_byte();
                self.alu_add(n);
                self.cycles += 8;
            }

            // ADC A, d8 — A = A + n + carry
            0xCE => {
                let n = self.fetch_byte();
                self.alu_adc(n);
                self.cycles += 8;
            }

            // SUB d8 — A = A - n
            0xD6 => {
                let n = self.fetch_byte();
                self.alu_sub(n);
                self.cycles += 8;
            }

            // SBC A, d8 — A = A - n - carry
            0xDE => {
                let n = self.fetch_byte();
                self.alu_sbc(n);
                self.cycles += 8;
            }

            // AND d8 — A = A & n
            0xE6 => {
                let n = self.fetch_byte();
                self.alu_and(n);
                self.cycles += 8;
            }

            // XOR d8 — A = A ^ n
            0xEE => {
                let n = self.fetch_byte();
                self.alu_xor(n);
                self.cycles += 8;
            }

            // OR d8 — A = A | n
            0xF6 => {
                let n = self.fetch_byte();
                self.alu_or(n);
                self.cycles += 8;
            }

            // CP d8 — compare A with n (sets flags from A - n, result discarded)
            0xFE => {
                let n = self.fetch_byte();
                self.alu_cp(n);
                self.cycles += 8;
            }

            // INC (HL) — increment the byte in memory at HL (Z/N/H flags, C unaffected)
            0x34 => {
                let value = self.read_operand(6); // (HL)
                let result = self.alu_inc(value);
                self.write_operand(6, result);
                self.cycles += 12;
            }

            // DEC (HL) — decrement the byte in memory at HL (Z/N/H flags, C unaffected)
            0x35 => {
                let value = self.read_operand(6); // (HL)
                let result = self.alu_dec(value);
                self.write_operand(6, result);
                self.cycles += 12;
            }

            // LD (HL), d8 — store immediate byte into memory at HL
            0x36 => {
                let n = self.fetch_byte();
                self.write_operand(6, n); // idx 6 = (HL)
                self.cycles += 12;
            }

            // SCF — set carry flag (C=1, N=0, H=0; Z unchanged)
            0x37 => {
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(true);
                self.cycles += 4;
            }

            // CCF — complement carry flag (C=!C, N=0, H=0; Z unchanged)
            0x3F => {
                let c = self.registers.flag_carry();
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(!c);
                self.cycles += 4;
            }

            // CPL — complement A (A = ~A; N=1, H=1; Z and C unchanged)
            0x2F => {
                self.registers.a = !self.registers.a;
                self.registers.set_flag_subtract(true);
                self.registers.set_flag_half_carry(true);
                self.cycles += 4;
            }

            // DAA — decimal adjust A to valid BCD after an add/sub, using N/H/C flags.
            // After addition (N=0): add 0x06 if a low-nibble carry happened, 0x60 if a high
            // one did. After subtraction (N=1): subtract the same. Sets Z, clears H, sets C.
            0x27 => {
                let mut a = self.registers.a;
                let mut carry = self.registers.flag_carry();
                if !self.registers.flag_subtract() {
                    if carry || a > 0x99 {
                        a = a.wrapping_add(0x60);
                        carry = true;
                    }
                    if self.registers.flag_half_carry() || (a & 0x0F) > 0x09 {
                        a = a.wrapping_add(0x06);
                    }
                } else {
                    if carry {
                        a = a.wrapping_sub(0x60);
                    }
                    if self.registers.flag_half_carry() {
                        a = a.wrapping_sub(0x06);
                    }
                }
                self.registers.a = a;
                self.registers.set_flag_zero(a == 0);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(carry);
                self.cycles += 4;
            }

            // Accumulator rotates. Like the CB-prefix RLC/RRC/RL/RR but A-only,
            // 1 byte, 4 cycles, and CRUCIALLY Z is ALWAYS cleared (N=0, H=0 too).

            // RLCA — rotate A left, circular. Old bit 7 -> carry and -> bit 0.
            0x07 => {
                let a = self.registers.a;
                let carry = a & 0x80 != 0;
                self.registers.a = a.rotate_left(1);
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(carry);
                self.cycles += 4;
            }

            // RRCA — rotate A right, circular. Old bit 0 -> carry and -> bit 7.
            0x0F => {
                let a = self.registers.a;
                let carry = a & 0x01 != 0;
                self.registers.a = a.rotate_right(1);
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(carry);
                self.cycles += 4;
            }

            // RLA — rotate A left through carry. Old carry -> bit 0; old bit 7 -> carry.
            0x17 => {
                let a = self.registers.a;
                let old_carry = self.registers.flag_carry() as u8;
                let new_carry = a & 0x80 != 0;
                self.registers.a = (a << 1) | old_carry;
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(new_carry);
                self.cycles += 4;
            }

            // RRA — rotate A right through carry. Old carry -> bit 7; old bit 0 -> carry.
            0x1F => {
                let a = self.registers.a;
                let old_carry = self.registers.flag_carry() as u8;
                let new_carry = a & 0x01 != 0;
                self.registers.a = (a >> 1) | (old_carry << 7);
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(new_carry);
                self.cycles += 4;
            }

            // LD SP, HL — copy HL into the stack pointer (no flags)
            0xF9 => {
                self.sp = self.registers.hl();
                self.cycles += 8;
            }

            // LD (a16), SP — store the 16-bit SP at an absolute address (little-endian)
            0x08 => {
                let addr = self.fetch_word();
                self.memory.write_word(addr, self.sp);
                self.cycles += 20;
            }

            // ADD SP, r8 — SP += signed 8-bit offset.
            // Z=0, N=0; H and C come from the UNSIGNED low-byte/low-nibble add, not the
            // 16-bit result. r8 is sign-extended for the actual addition.
            0xE8 => {
                let r8 = self.fetch_byte() as i8 as i16 as u16;
                let sp = self.sp;
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers
                    .set_flag_half_carry((sp & 0x0F) + (r8 & 0x0F) > 0x0F);
                self.registers.set_flag_carry((sp & 0xFF) + (r8 & 0xFF) > 0xFF);
                self.sp = sp.wrapping_add(r8);
                self.cycles += 16;
            }

            // LD HL, SP+r8 — HL = SP + signed 8-bit offset. Same flag rules as ADD SP,r8.
            0xF8 => {
                let r8 = self.fetch_byte() as i8 as i16 as u16;
                let sp = self.sp;
                self.registers.set_flag_zero(false);
                self.registers.set_flag_subtract(false);
                self.registers
                    .set_flag_half_carry((sp & 0x0F) + (r8 & 0x0F) > 0x0F);
                self.registers.set_flag_carry((sp & 0xFF) + (r8 & 0xFF) > 0xFF);
                let result = sp.wrapping_add(r8);
                self.registers.h = (result >> 8) as u8;
                self.registers.l = result as u8;
                self.cycles += 12;
            }

            // STOP: on CGB, performs an armed speed switch; otherwise stops the
            // CPU. It's a 2-byte opcode (0x10 0x00), so consume the next byte.
            0x10 => {
                self.fetch_byte();
                if !self.memory.try_speed_switch() {
                    self.halted = true;
                }
                self.cycles += 4;
            }

            //...so I need to implement all 256 opcodes?
            _ => panic!(
                "Unimplemented opcode: 0x{:02X} at PC: 0x{:04X}",
                opcode,
                self.pc - 1
            ),
        }
    }

    /// Decode and execute a CB-prefixed instruction.
    fn execute_cb(&mut self, opcode: u8) {
        let idx = opcode & 0x07; // bits 2-0: operand B,C,D,E,H,L,(HL),A
        let group = opcode >> 6; // bits 7-6: 0=rot/shift, 1=BIT, 2=RES, 3=SET
        let bit = (opcode >> 3) & 0x07; // bits 5-3: bit index (or rot/shift sub-op)

        match group {
            // rotate/shift family - sub-op selected by `bit`
            0 => {
                let value = self.read_operand(idx);
                let carry_in = self.registers.flag_carry();
                let (result, carry_out) = match bit {
                    0 => {
                        let c = value & 0x80 != 0;
                        ((value << 1) | c as u8, c)
                    } // RLC

                    1 => {
                        let c = value & 0x01 != 0;
                        ((value >> 1) | (c as u8) << 7, c)
                    } // RRC

                    2 => {
                        let c = value & 0x80 != 0;
                        ((value << 1) | carry_in as u8, c)
                    } // RL

                    3 => {
                        let c = value & 0x01 != 0;
                        ((value >> 1) | (carry_in as u8) << 7, c)
                    } // RR

                    4 => {
                        let c = value & 0x80 != 0;
                        (value << 1, c)
                    } // SLA

                    5 => {
                        let c = value & 0x01 != 0;
                        ((value >> 1) | (value & 0x80), c)
                    } // SRA

                    6 => (value.rotate_left(4), false), // SWAP

                    7 => {
                        let c = value & 0x01 != 0;
                        (value >> 1, c)
                    } // SRL

                    _ => unreachable!(),
                };
                self.write_operand(idx, result);
                self.registers.set_flag_zero(result == 0);
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(false);
                self.registers.set_flag_carry(carry_out);
                self.cycles += if idx == 6 { 16 } else { 8 };
            }

            // BIT b, r - test bit `bit` of operand; Z=!bit, N=0, H=1, C unaffected
            1 => {
                let value = self.read_operand(idx);
                let is_set = (value >> bit) & 1 == 1;
                self.registers.set_flag_zero(!is_set); // Z set when the tested bit is 0
                self.registers.set_flag_subtract(false);
                self.registers.set_flag_half_carry(true);
                // carry flag is left untouched
                self.cycles += if idx == 6 { 12 } else { 8 };
            }

            // RES b, r - clear bit `bit`
            2 => {
                let value = self.read_operand(idx);
                self.write_operand(idx, value & !(1 << bit));
                self.cycles += if idx == 6 { 16 } else { 8 };
            }

            // SET b, r - set bit `bit`
            3 => {
                let value = self.read_operand(idx);
                self.write_operand(idx, value | (1 << bit));
                self.cycles += if idx == 6 { 16 } else { 8 };
            }

            _ => unreachable!(),
        }
    }

    // ALU (Arithmetic Logic Unit) operations
    // Read an 8-bit operand by its 3-bit encoding (0=B ... 6=(HL) ... 7=A).
    fn read_operand(&self, idx: u8) -> u8 {
        match idx {
            0 => self.registers.b,
            1 => self.registers.c,
            2 => self.registers.d,
            3 => self.registers.e,
            4 => self.registers.h,
            5 => self.registers.l,
            6 => self.memory.read_byte(self.registers.hl()),
            7 => self.registers.a,
            _ => unreachable!(), // & 0x07 guarantees 0-7, but match must be exhaustive
        }
    }

    // Write an 8-bit operand by its 3-bit encoding (0=B ... 6=(HL) ... 7=A)
    fn write_operand(&mut self, idx: u8, value: u8) {
        match idx {
            0 => self.registers.b = value,
            1 => self.registers.c = value,
            2 => self.registers.d = value,
            3 => self.registers.e = value,
            4 => self.registers.h = value,
            5 => self.registers.l = value,
            6 => self.memory.write_byte(self.registers.hl(), value),
            7 => self.registers.a = value,
            _ => unreachable!(),
        }
    }

    //
    fn read_rr(&self, idx: u8, use_af: bool) -> u16 {
        match idx {
            0 => self.registers.bc(),
            1 => self.registers.de(),
            2 => self.registers.hl(),
            3 => {
                if use_af {
                    self.registers.af()
                } else {
                    self.sp
                }
            }
            _ => unreachable!(),
        }
    }

    fn write_rr(&mut self, idx: u8, use_af: bool, value: u16) {
        match idx {
            0 => self.registers.set_bc(value),
            1 => self.registers.set_de(value),
            2 => self.registers.set_hl(value),
            3 => {
                if use_af {
                    self.registers.set_af(value) // rp2 table -> AF
                } else {
                    self.sp = value; // rp table -> SP (a plain field assignment)
                }
            }
            _ => unreachable!(),
        }
    }

    // takes the 2-bit condiiton code from a conditional JP/JR/CALL/RET and returns
    // whether the branch should be taken. No mutation (it only reafs flags -> &self),
    // no break points, clean 4-way map
    // The encoding (bits 4-3 of those opcodes):
    // 0 -> NZ (take if Z flag is CLEAR)
    // 1 -> Z  (take if Z flag is SET)
    // 2 -> NC (take if Carry is CLEAR)
    // 3 -> C  (take if Carry is SET)
    fn check_condition(&self, cc: u8) -> bool {
        match cc {
            0 => !self.registers.flag_zero(),  // NZ
            1 => self.registers.flag_zero(),   // Z
            2 => !self.registers.flag_carry(), // NC
            3 => self.registers.flag_carry(),  // C
            _ => unreachable!(),
        }
    }

    fn alu_dispatch(&mut self, op: u8, value: u8) {
        match op {
            0 => self.alu_add(value),
            1 => self.alu_adc(value),
            2 => self.alu_sub(value),
            3 => self.alu_sbc(value),
            4 => self.alu_and(value),
            5 => self.alu_xor(value),
            6 => self.alu_or(value),
            7 => self.alu_cp(value),
            _ => unreachable!(),
        }
    }

    fn alu_inc(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);

        // Set flags
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(false);
        self.registers.set_flag_half_carry((value & 0x0F) == 0x0F);

        result
    }

    fn alu_dec(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);

        // Set flags
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(true);
        self.registers.set_flag_half_carry((value & 0x0F) == 0);

        result
    }

    fn alu_add(&mut self, value: u8) {
        let a = self.registers.a;
        let result = a.wrapping_add(value);

        // Set flags
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(false);
        self.registers
            .set_flag_half_carry((a & 0x0F) + (value & 0x0F) > 0x0F);
        self.registers
            .set_flag_carry(a as u16 + value as u16 > 0xFF);

        self.registers.a = result;
    }

    fn alu_adc(&mut self, value: u8) {
        let a = self.registers.a;
        let carry = self.registers.flag_carry() as u8; /* flag_carry() returns a bool, so false as u8 -> 0, true as u8 -> 1*/

        let result = a.wrapping_add(value).wrapping_add(carry); /* wrapping_add is a method on u8, plain + on u8 panics in debug builds on overflow (e.g. 255+1).*/
        /* we also chain (a + value + carry) mod 256 */
        // Set flags
        self.registers.set_flag_zero(result == 0); // evaluates to a bool, which is what the
        // setter takes. zero flag is set when the 8-bit result is zero

        self.registers.set_flag_subtract(false);
        // ADC is addition, so the N flag is always cleared. Literal false.
        self.registers
            .set_flag_half_carry((a & 0x0F) + (value & 0x0F) + carry > 0x0F);
        // a & 0x0F masks off everything but the low nibble (bottom 4 bits). & is bitwise-AND here.
        // a, value, and carry are all u8. The masked values are at most 0x0F (15) each, plus carry
        // <= 1, so the sum amxes at 15 + 15 + 1 = 31, comfortably under 255, so this u8 addition
        // can't overflow and wrapping/widening isn't needed here. The whole expression is a bool (>
        // (0x0F), passed straight to the setter
        self.registers
            .set_flag_carry((a as u16) + (value as u16) + (carry as u16) > 0xFF);
        // We must widen to u16 first. If we added these as u7, 255+1 would wrap to 0, and we would
        // lose the very overflow we're trying to detect. By casting to u16 (range 0-65535), the sum
        // 255 + 0 + 1 = 256 survives intact, and 256 > 0xFF is true.
        // That's the carry
        // Note, we cast the inputs and add in u16 space, not result, which has already wrapped
        self.registers.a = result;
    }

    fn alu_sub(&mut self, value: u8) {
        let a = self.registers.a;
        let result = a.wrapping_sub(value);

        // Set flags
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(true);
        self.registers
            .set_flag_half_carry((a & 0x0F) < (value & 0x0F));
        self.registers.set_flag_carry(a < value);

        self.registers.a = result;
    }

    fn alu_sbc(&mut self, value: u8) {
        let a = self.registers.a;
        let carry = self.registers.flag_carry() as u8; // read BEFORE setting flags -same rule
        // as ADC

        let result = a.wrapping_sub(value).wrapping_sub(carry);

        // Set flags
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(true); // it's subtract -> N = 1
        self.registers
            .set_flag_half_carry((a & 0x0F) < (value & 0x0F) + carry); // low nibble-borrow, including
        // carry
        self.registers
            .set_flag_carry((a as u16) < (value as u16) + (carry as u16)); // full borrow, including carry

        self.registers.a = result;
    }

    fn alu_and(&mut self, value: u8) {
        let result = self.registers.a & value; // bitwise AND
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(false);
        self.registers.set_flag_half_carry(true); // the quirk, ALWAYS true for AND
        self.registers.set_flag_carry(false);
        self.registers.a = result;
    }

    fn alu_xor(&mut self, value: u8) {
        let result = self.registers.a ^ value; // ^ is bitwise XOR
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(false);
        self.registers.set_flag_half_carry(false); // XOR clears H ( no quirk, unlike AND)
        self.registers.set_flag_carry(false);
        self.registers.a = result;
    }

    fn alu_or(&mut self, value: u8) {
        let result = self.registers.a | value; // | is bitwise OR
        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(false);
        self.registers.set_flag_half_carry(false);
        self.registers.set_flag_carry(false);
        self.registers.a = result;
    }

    fn alu_cp(&mut self, value: u8) {
        let a = self.registers.a;
        let result = a.wrapping_sub(value);

        self.registers.set_flag_zero(result == 0);
        self.registers.set_flag_subtract(true);
        self.registers
            .set_flag_half_carry((a & 0x0F) < (value & 0x0F));
        self.registers.set_flag_carry(a < value);
        // NOTE: no self.registers.a = result; CP discards the result, A is unchanged
    }
}

/// Decode one 8-pixel tile row (2 bytes, 2bpp) into 8 color IDs (0-3)
fn decode_tile_row(low: u8, high: u8) -> [u8; 8] {
    let mut pixels = [0u8; 8];
    for i in 0..8 {
        let bit = 7 - i; // leftmost pixel = bit 7
        let lo = (low >> bit) & 1;
        let hi = (high >> bit) & 1;
        pixels[i] = (hi << 1) | lo;
    }
    pixels
}

/// Map a 2-bit color id through the palette register to a 2-bit shade.
fn apply_palette(palette: u8, color: u8) -> u8 {
    (palette >> (color * 2)) & 0b11
}

/// Convert a DMG 2-bit shade to an (R,G,B) pixel (classic Game Boy green).
/// This is where DMG monochrome becomes real RGB; CGB will use its own palettes.
fn dmg_rgb(shade: u8) -> (u8, u8, u8) {
    match shade {
        0 => (224, 248, 208),
        1 => (136, 192, 112),
        2 => (52, 104, 86),
        _ => (8, 24, 32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_pairing() {
        let mut registers = Registers::new();

        // Test BC pairing
        registers.b = 0x12;
        registers.c = 0x34;
        assert_eq!(registers.bc(), 0x1234);

        registers.set_bc(0xABCD);
        assert_eq!(registers.b, 0xAB);
        assert_eq!(registers.c, 0xCD);

        // Test DE pairing
        registers.d = 0x56;
        registers.e = 0x78;
        assert_eq!(registers.de(), 0x5678);

        registers.set_de(0xEF01);
        assert_eq!(registers.d, 0xEF);
        assert_eq!(registers.e, 0x01);

        // Test HL pairing
        registers.h = 0x9A;
        registers.l = 0xBC;
        assert_eq!(registers.hl(), 0x9ABC);

        registers.set_hl(0x2345);
        assert_eq!(registers.h, 0x23);
        assert_eq!(registers.l, 0x45);
    }

    #[test]
    fn test_flag_operations() {
        let mut registers = Registers::new();
        registers.f = 0; // clean slate (post-boot default is 0xB0); this test checks setters

        // All flags should now be clear
        assert_eq!(registers.f, 0);
        assert!(!registers.flag_zero());
        assert!(!registers.flag_subtract());
        assert!(!registers.flag_half_carry());
        assert!(!registers.flag_carry());

        // Test zero flag
        registers.set_flag_zero(true);
        assert!(registers.flag_zero());
        assert_eq!(registers.f & 0b1000_0000, 0b1000_0000);

        registers.set_flag_zero(false);
        assert!(!registers.flag_zero());

        // Test subtract flag
        registers.set_flag_subtract(true);
        assert!(registers.flag_subtract());
        assert_eq!(registers.f & 0b0100_0000, 0b0100_0000);

        // Test half carry flag
        registers.set_flag_half_carry(true);
        assert!(registers.flag_half_carry());
        assert_eq!(registers.f & 0b0010_0000, 0b0010_0000);

        // Test carry flag
        registers.set_flag_carry(true);
        assert!(registers.flag_carry());
        assert_eq!(registers.f & 0b0001_0000, 0b0001_0000);

        // Test multiple flags at once
        registers.f = 0;
        registers.set_flag_zero(true);
        registers.set_flag_carry(true);
        assert!(registers.flag_zero());
        assert!(registers.flag_carry());
        assert!(!registers.flag_subtract());
        assert_eq!(registers.f, 0b1001_0000);
    }

    #[test]
    fn test_flag_lower_bits_always_zero() {
        let mut registers = Registers::new();

        // Set all flags
        registers.set_flag_zero(true);
        registers.set_flag_subtract(true);
        registers.set_flag_half_carry(true);
        registers.set_flag_carry(true);

        // Lower 4 bits should still be zero
        assert_eq!(registers.f & 0x0F, 0);
    }

    fn test_register_enum_get_set() {
        let mut cpu = Cpu::new();

        // Test that the enum works as expected
        Register::B.set(&mut cpu, 0x12);
        assert_eq!(Register::B.get(&cpu), 0x12);
        assert_eq!(cpu.registers.b, 0x12);

        Register::A.set(&mut cpu, 0xFF);
        assert_eq!(Register::A.get(&cpu), 0xFF);
        assert_eq!(cpu.registers.a, 0xFF);
    }
}

#[cfg(test)]
mod instruction_tests {
    use super::*;

    // Helper function to create a CPU with a program loaded
    fn setup_cpu(program: Vec<u8>) -> Cpu {
        let mut cpu = Cpu::new();
        cpu.memory.load_rom(&program);
        cpu.pc = 0x0000; // Start from beginning for tests
        cpu
    }

    #[test]
    fn test_ld_r_n_family() {
        struct TestCase {
            opcode: u8,
            value: u8,
            register: Register,
        }

        let cases = vec![
            TestCase {
                opcode: 0x06,
                value: 0x12,
                register: Register::B,
            },
            TestCase {
                opcode: 0x0E,
                value: 0x34,
                register: Register::C,
            },
            TestCase {
                opcode: 0x16,
                value: 0x56,
                register: Register::D,
            },
            TestCase {
                opcode: 0x1E,
                value: 0x78,
                register: Register::E,
            },
            TestCase {
                opcode: 0x26,
                value: 0x9A,
                register: Register::H,
            },
            TestCase {
                opcode: 0x2E,
                value: 0xBC,
                register: Register::L,
            },
            TestCase {
                opcode: 0x3E,
                value: 0xDE,
                register: Register::A,
            },
        ];

        for case in cases {
            let mut cpu = setup_cpu(vec![case.opcode, case.value]);
            cpu.step();
            assert_eq!(
                case.register.get(&cpu),
                case.value,
                "Failed for LD {}, 0x{:02X}",
                case.register.name(),
                case.value
            );
            assert_eq!(cpu.cycles, 8)
        }
    }

    #[test]
    fn test_ld_r_r_family() {
        // Test LD r, r' instructions (0x40-0x7F, except 0x76 HALT)

        for dest in Register::all() {
            for src in Register::all() {
                // Calculate opcode: 0x40 + (dest * 8) + src
                let dest_idx = match dest {
                    Register::B => 0,
                    Register::C => 1,
                    Register::D => 2,
                    Register::E => 3,
                    Register::H => 4,
                    Register::L => 5,
                    Register::A => 7,
                };

                let src_idx = match src {
                    Register::B => 0,
                    Register::C => 1,
                    Register::D => 2,
                    Register::E => 3,
                    Register::H => 4,
                    Register::L => 5,
                    Register::A => 7,
                };

                let opcode = 0x40 + (dest_idx * 8) + src_idx;
                let test_value = 0x42 + src_idx;

                let mut cpu = setup_cpu(vec![opcode]);

                // Set source register
                src.set(&mut cpu, test_value);

                cpu.step();

                // Check destination register
                assert_eq!(
                    dest.get(&cpu),
                    test_value,
                    "Failed for LD {}, {} (opcode 0x{:02X})",
                    dest.name(),
                    src.name(),
                    opcode
                );

                assert_eq!(cpu.cycles, 4);
            }
        }
    }

    #[test]
    fn test_ld_rr_d16() {
        // LD SP, 0xFFFE - opcode 0x31, then the 16-bit value little-endian (low byte first)
        let mut cpu = setup_cpu(vec![0x31, 0xFE, 0xFF]);
        cpu.step();
        assert_eq!(cpu.sp, 0xFFFE); // the two bytes reassembled into the word
        assert_eq!(cpu.cycles, 12);

        // LD, BC 0x1234 - opcode 0x01, value bytes 0x34 then 0x12
        let mut cpu = setup_cpu(vec![0x01, 0x34, 0x12]);
        cpu.step();
        assert_eq!(cpu.registers.bc(), 0x1234);
    }

    #[test]
    fn test_inc_dec_rr() {
        // INC BC normally
        let mut cpu = setup_cpu(vec![0x03]); // INC BC
        cpu.registers.set_bc(0x1234);
        cpu.step();
        assert_eq!(cpu.registers.bc(), 0x1235);
        assert_eq!(cpu.cycles, 8);

        // INC wraps 0xFFFF -> 0x0000 AND sets NO flags (Z must stay clear)
        let mut cpu = setup_cpu(vec![0x23]); // INC HL
        cpu.registers.set_hl(0xFFFF);
        cpu.registers.set_flag_zero(false); // clear Z first so we can prove INC rr never sets it
        cpu.step();
        assert_eq!(cpu.registers.hl(), 0x0000);
        assert!(!cpu.registers.flag_zero()); // result is 0 but Z is NOT set

        // DEC wraps 0x0000 -> 0xFFFF
        let mut cpu = setup_cpu(vec![0x0B]); // DEC BC
        cpu.registers.set_bc(0x0000);
        cpu.step();
        assert_eq!(cpu.registers.bc(), 0xFFFF);
    }

    #[test]
    fn test_add_hl_rr() {
        // half-carry at bit 11: 0x0FFF + 0x0001 = 0x1000
        let mut cpu = setup_cpu(vec![0x09]); // ADD HL, BC
        cpu.registers.set_hl(0x0FFF);
        cpu.registers.set_bc(0x0001);
        cpu.step();
        assert_eq!(cpu.registers.hl(), 0x1000);
        assert!(cpu.registers.flag_half_carry()); //bit-11 carry
        assert!(!cpu.registers.flag_carry()); // no bit-15 carry
        assert!(!cpu.registers.flag_subtract());

        // full carry at bit 15, AND Z must survice
        let mut cpu = setup_cpu(vec![0x09]); // ADD HL, BC
        cpu.registers.set_hl(0xFFFF);
        cpu.registers.set_bc(0x0002);
        cpu.registers.set_flag_zero(true); // set Z BEFORE the add
        cpu.step();
        assert_eq!(cpu.registers.hl(), 0x0001);
        assert!(cpu.registers.flag_carry());
        assert!(cpu.registers.flag_zero()); // Z untouched, even though result is 0x0000
    }

    #[test]
    fn test_ld_a_rr() {
        // LD A, (BC) - read a byte BC points at
        let mut cpu = setup_cpu(vec![0x0A]); // LD A, (BC)
        cpu.memory.write_byte(0xC000, 0x42); // plant a value in WRAM
        cpu.registers.set_bc(0xC000);
        cpu.step();
        assert_eq!(cpu.registers.a, 0x42);

        // LD A, (HL+) - read, then HL must increment
        let mut cpu = setup_cpu(vec![0x2A]); // LD A, (HL+)
        cpu.memory.write_byte(0xC000, 0x99);
        cpu.registers.set_hl(0xC000);
        cpu.step();
        assert_eq!(cpu.registers.a, 0x99);
        assert_eq!(cpu.registers.hl(), 0xC001); // HL advanced past the byte it just read
    }

    #[test]
    fn test_jp_cc_not_taken_still_advances_pc() {
        // condition FALSE: Z set, but JP NZ wants Z clear -> not taken
        let mut cpu = setup_cpu(vec![0xC2, 0x34, 0x12]);
        cpu.registers.set_flag_zero(true);
        cpu.step();
        assert_eq!(cpu.pc, 3); // consumed opcode + 2 operand bytes
        assert_eq!(cpu.cycles, 12); // not-taken cost
        // assert pc == 3
        // // assert cycles == 12
    }

    #[test]
    fn test_jp_cc_taken() {
        // condition TRUE: Z clear, JP NZ takes the branch
        let mut cpu = setup_cpu(vec![0xC2, 0x34, 0x12]);
        cpu.registers.set_flag_zero(false);
        cpu.step();
        assert_eq!(cpu.pc, 0x1234); // jumpted to target
        assert_eq!(cpu.cycles, 16); // taken cost
        // assert pc == 0x1234
        // assert cycles == 16
    }

    #[test]
    fn test_jr_negative_offset() {
        // JR -2 (0xFE as i8 == -2)
        let mut cpu = setup_cpu(vec![0x18, 0xFE]);
        cpu.step();
        assert_eq!(cpu.pc, 0); // 2+ (-2); would be 256 if cast were unsigned
        assert_eq!(cpu.cycles, 12);
    }

    #[test]
    fn test_jr_cc_not_taken() {
        let mut cpu = setup_cpu(vec![0x20, 0x05]);
        cpu.registers.set_flag_zero(true);
        cpu.step();
        assert_eq!(cpu.pc, 2);
        assert_eq!(cpu.cycles, 8);
    }

    #[test]
    fn test_push_pop_roundstrip() {
        let mut cpu = setup_cpu(vec![0xC5, 0xD1]);
        cpu.registers.set_bc(0xBEEF);
        let sp_before = cpu.sp;

        cpu.step(); // PUSH BC
        assert_eq!(cpu.sp, sp_before.wrapping_sub(2)); //stack grew down by 2

        cpu.step(); // POP DE
        assert_eq!(cpu.registers.de(), 0xBEEF); // value survived the round-trip
        assert_eq!(cpu.sp, sp_before); // sp balanced back to start
    }

    #[test]
    fn test_call_ret_roundtrip() {
        // CALL 0x0005 at addr 0; RET sits at addr 5
        let mut cpu = setup_cpu(vec![0xCD, 0x05, 0x00, 0x00, 0x00, 0xC9]);
        let sp_before = cpu.sp;

        cpu.step(); // CALL 0x0005
        assert_eq!(cpu.pc, 0x0005); // jumped into the subroutine
        assert_eq!(cpu.sp, sp_before.wrapping_sub(2)); // pushed 2 bytes
        assert_eq!(cpu.memory.read_word(cpu.sp), 3); // return address == 3 (instr after CALL)

        cpu.step(); // RET
        assert_eq!(cpu.pc, 3); // resumed right after the CALL
        assert_eq!(cpu.sp, sp_before); // stack balanced backL
    }

    #[test]
    fn test_ret_cc_not_taken() {
        let mut cpu = setup_cpu(vec![0xC0]); // RET NZ
        cpu.registers.set_flag_zero(true); // Z set -> NZ false -> NOT taken
        let sp_before = cpu.sp;
        cpu.step();
        assert_eq!(cpu.pc, 1); // only the opcode consumed; no stack jump
        assert_eq!(cpu.sp, sp_before); // stack untouched, we did NOT pop
        assert_eq!(cpu.cycles, 8); // not-taken cost
    }

    #[test]
    fn test_ret_pushes_and_jumps() {
        let mut cpu = setup_cpu(vec![0xEF]); // RST 0x28
        let sp_before = cpu.sp;
        cpu.step();
        assert_eq!(cpu.pc, 0x28); // jumped to vector 5*8 = 0x28
        assert_eq!(cpu.sp, sp_before.wrapping_sub(2)); // pushed 2 bytes
        assert_eq!(cpu.memory.read_word(cpu.sp), 1); // return addr = 1 (byte after the 1-byte RST)
    }

    #[test]
    fn test_cb_bit() {
        // BIT 7, A (0xCB, 0x7F) with bit 7 SET -> Z should be CLEAR
        let mut cpu = setup_cpu(vec![0xCB, 0x7F]);
        cpu.registers.a = 0x80; // bit 7 = 1 
        cpu.registers.set_flag_carry(true); // plant a carry to prove it survives
        cpu.step();
        assert!(!cpu.registers.flag_zero()); // bit is set -> Z clear (key inversion)
        assert!(!cpu.registers.flag_subtract()); // N = 0
        assert!(cpu.registers.flag_half_carry()); // H = 1
        assert!(cpu.registers.flag_carry()); // C preserved (never touched)
        assert_eq!(cpu.cycles, 8);

        // BIT 7, A with bit 7 CLEAR -> Z should be SET
        let mut cpu = setup_cpu(vec![0xCB, 0x7F]);
        cpu.registers.a = 0x00;
        cpu.step();
        assert!(cpu.registers.flag_zero()); // bit is clear -> Z set
    }

    #[test]
    fn test_cb_set_res() {
        // SET 3, B with B = 0x00 -> only bit 3 turns on
        let mut cpu = setup_cpu(vec![0xCB, 0xD8]);
        cpu.registers.b = 0x00;
        cpu.step();
        assert_eq!(cpu.registers.b, 0x08); // 0000_1000
        assert_eq!(cpu.cycles, 8); // register operand

        // RES 3, B with B = 0xFF -> only bit 3 turns off, neighbors survive
        let mut cpu = setup_cpu(vec![0xCB, 0x98]);
        cpu.registers.b = 0xFF;
        cpu.step();
        assert_eq!(cpu.registers.b, 0xF7); // 1111_0111, The discriminating assert
    }

    #[test]
    fn test_cb_rl_and_sra() {
        // RL B: the carry flag flows INTO bit 0
        let mut cpu = setup_cpu(vec![0xCB, 0x10]);
        cpu.registers.b = 0x00;
        cpu.registers.set_flag_carry(true); // this 1 should land in bit 0
        cpu.step();
        assert_eq!(cpu.registers.b, 0x01); // carry_in -> bit 0 (RLC would give 0x00)
        assert!(!cpu.registers.flag_carry()); // old bit 8 was 0 -> carry out clear
        assert!(!cpu.registers.flag_zero()); // result is nonzero

        // SRA A: the sign bit (bit 7) is preserved
        let mut cpu = setup_cpu(vec![0xCB, 0x2F]);
        cpu.registers.a = 0x80;
        cpu.step();
        assert_eq!(cpu.registers.a, 0xC0); // bit 7 stays set (SRL would give 0x40)
        assert!(!cpu.registers.flag_carry()); // old bit 0 was 0
    }

    #[test]
    fn test_inc_r_family() {
        struct TestCase {
            opcode: u8,
            register: Register,
        }

        let cases = vec![
            TestCase {
                opcode: 0x04,
                register: Register::B,
            },
            TestCase {
                opcode: 0x0C,
                register: Register::C,
            },
            TestCase {
                opcode: 0x14,
                register: Register::D,
            },
            TestCase {
                opcode: 0x1C,
                register: Register::E,
            },
            TestCase {
                opcode: 0x24,
                register: Register::H,
            },
            TestCase {
                opcode: 0x2C,
                register: Register::L,
            },
            TestCase {
                opcode: 0x3C,
                register: Register::A,
            },
        ];

        for case in cases {
            // Test normal increment
            let mut cpu = setup_cpu(vec![case.opcode]);
            case.register.set(&mut cpu, 0x42);
            cpu.step();
            assert_eq!(
                case.register.get(&cpu),
                0x43,
                "Failed normal INC {}",
                case.register.name()
            );
            assert!(!cpu.registers.flag_zero());

            // Test increment with half-carry (0x0F -> 0x10)
            let mut cpu = setup_cpu(vec![case.opcode]);
            case.register.set(&mut cpu, 0x0F);
            cpu.step();
            assert_eq!(case.register.get(&cpu), 0x10);
            assert!(
                cpu.registers.flag_half_carry(),
                "Failed zero flag for INC {}",
                case.register.name()
            );

            // Test increment with zero (0xFF -> 0x00)
            let mut cpu = setup_cpu(vec![case.opcode]);
            case.register.set(&mut cpu, 0xFF);
            cpu.step();
            assert_eq!(case.register.get(&cpu), 0x00);
            assert!(
                cpu.registers.flag_zero(),
                "Failed zero flag for INC {}",
                case.register.name()
            );
        }
    }

    #[test]
    fn test_dec_r_family() {
        struct TestCase {
            opcode: u8,
            register: Register,
        }

        let cases = vec![
            TestCase {
                opcode: 0x05,
                register: Register::B,
            },
            TestCase {
                opcode: 0x0D,
                register: Register::C,
            },
            TestCase {
                opcode: 0x15,
                register: Register::D,
            },
            TestCase {
                opcode: 0x1D,
                register: Register::E,
            },
            TestCase {
                opcode: 0x25,
                register: Register::H,
            },
            TestCase {
                opcode: 0x2D,
                register: Register::L,
            },
            TestCase {
                opcode: 0x3D,
                register: Register::A,
            },
        ];

        for case in cases {
            // Test normal increment
            let mut cpu = setup_cpu(vec![case.opcode]);
            case.register.set(&mut cpu, 0x42);
            cpu.step();
            assert_eq!(
                case.register.get(&cpu),
                0x41,
                "Failed normal DEC {}",
                case.register.name()
            );
            assert!(!cpu.registers.flag_zero());
            assert!(cpu.registers.flag_subtract());

            // Test decrement to zero
            let mut cpu = setup_cpu(vec![case.opcode]);
            case.register.set(&mut cpu, 0x01);
            cpu.step();
            assert_eq!(case.register.get(&cpu), 0x00);
            assert!(
                cpu.registers.flag_zero(),
                "Failed zero flag for DEC {}",
                case.register.name()
            );
        }
    }

    #[test]
    fn test_add_a_r_family() {
        // Test ADD with different registers (0x80-0x86)
        for (idx, reg) in Register::all()[..6].iter().enumerate() {
            // Skip the last one (A)
            let opcode = 0x80 + idx as u8;

            // Test normal addition
            let mut cpu = setup_cpu(vec![opcode]);
            Register::A.set(&mut cpu, 0x05);
            reg.set(&mut cpu, 0x03);
            cpu.step();
            assert_eq!(Register::A.get(&cpu), 0x08, "Failed ADD A,{}", reg.name());

            // Test with carry
            let mut cpu = setup_cpu(vec![opcode]);
            Register::A.set(&mut cpu, 0xFF);
            reg.set(&mut cpu, 0x02);
            cpu.step();
            assert_eq!(Register::A.get(&cpu), 0x01);
            assert!(
                cpu.registers.flag_carry(),
                "Failed carry for ADD A,{}",
                reg.name()
            );
        }
    }

    #[test]
    fn test_add_a_a() {
        // Special case: ADD A, A (0x87)
        let mut cpu = setup_cpu(vec![0x87]);
        Register::A.set(&mut cpu, 0x05);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0x0A); // 0x05 + 0x05 = 0x0A

        // Test with carry
        let mut cpu = setup_cpu(vec![0x87]);
        Register::A.set(&mut cpu, 0xFF);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0xFE); // 0xFF + 0xFF = 0x1FE -> 0xFE
        assert!(cpu.registers.flag_carry());
    }

    #[test]
    fn test_adc_a_b() {
        // making sure carry-in actually participates
        let mut cpu = setup_cpu(vec![0x88]);
        Register::A.set(&mut cpu, 0xFF);
        Register::B.set(&mut cpu, 0x00);
        cpu.registers.set_flag_carry(true); // set the carry flag before stepping
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0x00);
        assert!(cpu.registers.flag_zero());
        assert!(cpu.registers.flag_carry());
        assert!(cpu.registers.flag_half_carry());
        assert!(!cpu.registers.flag_subtract());
    }

    #[test]
    fn test_sub_b() {
        // 0x05 - 0x10 wraps to 0xF5
        let mut cpu = setup_cpu(vec![0x90]); // set to SUB B opcode 
        Register::A.set(&mut cpu, 0x05);
        Register::B.set(&mut cpu, 0x10);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0xF5);
        assert!(cpu.registers.flag_carry());
        assert!(!cpu.registers.flag_half_carry()); // low-nibble borrow 0x5 < 0x0, but (0x05 & 0x0F)
        // < (0x10 & 0x0F), 5<0? What should H be
        assert!(cpu.registers.flag_subtract());
        assert!(!cpu.registers.flag_zero());
    }

    #[test]
    fn test_sbc_a_b() {
        let mut cpu = setup_cpu(vec![0x98]); //SBC A,B
        Register::A.set(&mut cpu, 0x00);
        Register::B.set(&mut cpu, 0x00);
        cpu.registers.set_flag_carry(true); // the borrowin, the whole point
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0xFF);
        assert!(cpu.registers.flag_carry());
        assert!(cpu.registers.flag_half_carry());
        assert!(cpu.registers.flag_subtract());
        assert!(!cpu.registers.flag_zero());
    }

    #[test]
    fn test_and_b() {
        let mut cpu = setup_cpu(vec![0xA0]); // AND B
        Register::A.set(&mut cpu, 0x0F);
        Register::B.set(&mut cpu, 0xF0);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0x00); // 0x0F & 0xF0 = 0x00 ( no overlapping bits)
        assert!(cpu.registers.flag_zero()); // result is zero
        assert!(cpu.registers.flag_half_carry()); //The QUIRK - H set even though nothing carried
        assert!(!cpu.registers.flag_carry()); // AND always clears C
        assert!(!cpu.registers.flag_subtract()) // not a subtract
    }

    #[test]
    fn test_xor_a() {
        let mut cpu = setup_cpu(vec![0xAF]); // XOR A
        Register::A.set(&mut cpu, 0xFF); // any value
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0x00); // XOR'd with itself is always 0
        assert!(cpu.registers.flag_zero()); // so Z must be set
        assert!(!cpu.registers.flag_half_carry()); // XOR clears H (NOT AND's quirk)
        assert!(!cpu.registers.flag_carry());
        assert!(!cpu.registers.flag_subtract());
    }

    #[test]
    fn test_or_b() {
        let mut cpu = setup_cpu(vec![0xB0]); // OR B
        Register::A.set(&mut cpu, 0xF0);
        Register::B.set(&mut cpu, 0x0F);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0xFF); // 0xF0 | 0x0F = 0xFF (bits combine)
        assert!(!cpu.registers.flag_zero()); // result non-zero -> Z clear
        assert!(!cpu.registers.flag_half_carry());
        assert!(!cpu.registers.flag_carry());
        assert!(!cpu.registers.flag_subtract());
    }

    #[test]
    fn test_cp_b() {
        let mut cpu = setup_cpu(vec![0xB8]); // CP B
        Register::A.set(&mut cpu, 0x05);
        Register::B.set(&mut cpu, 0x05);
        cpu.step();
        assert_eq!(Register::A.get(&cpu), 0x05); // THE POINT: A is NOT modified
        assert!(cpu.registers.flag_zero()); // 0x0f == 0x0f -> equal -> Z set
        assert!(cpu.registers.flag_subtract());
        assert!(!cpu.registers.flag_carry()); // A not less than B
    }

    #[test]
    fn test_write_rr_roundtrip() {
        let mut cpu = Cpu::new();
        cpu.write_rr(2, false, 0x1234); // write HL
        assert_eq!(cpu.read_rr(2, false), 0x1234); // read it back
        cpu.write_rr(3, false, 0xFFFE); // index 3, rp - SP
        assert_eq!(cpu.sp, 0xFFFE);
        cpu.write_rr(3, true, 0x12F0); // index 3, rp2 -> AF
        assert_eq!(cpu.registers.af(), 0x12F0);
    }

    #[test]
    fn test_check_condiiton() {
        let mut cpu = Cpu::new();

        // Zer set, Carry clear
        cpu.registers.set_flag_zero(true);
        cpu.registers.set_flag_carry(false);
        assert!(!cpu.check_condition(0)); // NZ: Z is set, so DON'T take
        assert!(cpu.check_condition(1)); // Z: Z is set, so take
        assert!(cpu.check_condition(2)); // NC: Carry clear, so take
        assert!(!cpu.check_condition(3)); // C: Carry clear, so DON'T take
    }

    #[test]
    fn test_ei_di() {
        // EI enables IME only AFTER the following instruction. Program: EI; NOP.
        let mut cpu = setup_cpu(vec![0xFB, 0x00]);
        cpu.interrupts_enabled = false;
        cpu.step(); // EI: still off (delay)
        assert!(!cpu.interrupts_enabled, "EI must not enable IME immediately");
        cpu.step(); // NOP: now IME turns on
        assert!(cpu.interrupts_enabled, "IME enabled after the instruction post-EI");

        // DI clears IME
        let mut cpu = setup_cpu(vec![0xF3]);
        cpu.interrupts_enabled = true;
        cpu.step();
        assert!(!cpu.interrupts_enabled);
    }

    #[test]
    fn test_halt_bug() {
        // HALT with IME off and an interrupt already pending: the CPU doesn't
        // halt; PC fails to advance so the next opcode runs twice.
        let mut cpu = setup_cpu(vec![0x76, 0x3C]); // HALT; INC A
        cpu.interrupts_enabled = false;
        cpu.registers.a = 0;
        cpu.memory.write_byte(0xFFFF, 0x01); // IE: VBlank enabled
        cpu.memory.write_byte(0xFF0F, 0x01); // IF: VBlank pending

        cpu.step(); // HALT -> bug armed, not halted
        assert!(!cpu.halted);
        cpu.step(); // INC A (PC did not advance past it)
        cpu.step(); // INC A again (byte re-read)
        assert_eq!(cpu.registers.a, 2);
    }

    #[test]
    fn test_interrupt_dispatch() {
        let mut cpu = setup_cpu(vec![]);
        cpu.pc = 0x0150;
        cpu.interrupts_enabled = true;
        cpu.memory.write_byte(0xFFFF, 0b0000_0100); // IE: enable Timer (bit 2)
        cpu.memory.write_byte(0xFF0F, 0b0000_0100); // IF: request Timer
        let sp_before = cpu.sp;

        cpu.handle_interrupts();

        assert_eq!(cpu.pc, 0x0050); // jumpted to Timer vector (0x40 + 2*8)
        assert!(!cpu.interrupts_enabled); // IME cleared during handler
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x04, 0); // IF bit 2 acknowledged (cleared)
        assert_eq!(cpu.sp, sp_before.wrapping_sub(2)); // pushed return address
        assert_eq!(cpu.memory.read_word(cpu.sp), 0x0150); // return addr == old pc
        assert_eq!(cpu.cycles, 20);
    }

    #[test]
    fn test_interrupt_ime_off_wakes_halt_no_dispatch() {
        let mut cpu = setup_cpu(vec![]);
        cpu.pc = 0x0150;
        cpu.halted = true;
        cpu.interrupts_enabled = false; // IME OFF
        cpu.memory.write_byte(0xFFFF, 0b0000_0100); // enable Timer
        cpu.memory.write_byte(0xFF0F, 0b0000_0100); // request Timer
        cpu.handle_interrupts();

        assert!(!cpu.halted); // woke from HALF
        assert_eq!(cpu.pc, 0x0150); // .. but did NOT dispatch
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x04, 0x04); // IF still set (not acknowledged)
    }

    #[test]
    fn test_timer_overflow_requests_interrupt() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF07, 0b0000_0101); // TAC: enabled (bit2), period 16
        cpu.memory.write_byte(0xFF06, 0xAB); // TMA = reload value
        cpu.memory.write_byte(0xFF05, 0xFF); // TIMA one tick from overflow

        cpu.step_timer(16); // exactly one tick at period 16

        assert_eq!(cpu.memory.read_byte(0xFF05), 0xAB); // reloaded from TMA (NOT 0x00)
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x04, 0x04); // Time interrupt requested (IF bit 2)
    }

    #[test]
    fn test_timer_counts_without_overflow() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF07, 0b0000_0101); // enabled, period 16
        cpu.memory.write_byte(0xFF05, 0x00);

        cpu.step_timer(16);
        assert_eq!(cpu.memory.read_byte(0xFF05), 0x01); // ticked once
        assert_eq!(cpu.memory.read_byte(0xFF05) & 0x04, 0x00); // no interrupt yet
    }

    #[test]
    fn test_timer_disabled_does_not_count() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF07, 0b0000_0001); // period bits set, but ENABLE
        // (bit 2) CLEAR
        cpu.memory.write_byte(0xFF05, 0x42);
        cpu.step_timer(1000);

        assert_eq!(cpu.memory.read_byte(0xFF05), 0x42); // unchanged - timer is off
    }

    #[test]
    fn test_ppu_advances_ly_each_scanline() {
        let mut cpu = setup_cpu(vec![]);
        assert_eq!(cpu.memory.read_byte(0xFF44), 0x00);

        cpu.step_ppu(456); // exactly one scanline
        assert_eq!(cpu.memory.read_byte(0xFF44), 1);

        cpu.step_ppu(456); // another scanline
        assert_eq!(cpu.memory.read_byte(0xFF44), 2);
    }

    #[test]
    fn test_ppu_requests_vblank_at_line_144() {
        let mut cpu = setup_cpu(vec![]);
        // drive the beam from LY=0 up to the start of VBlank (line 144)
        cpu.step_ppu(456 * 144);

        assert_eq!(cpu.memory.read_byte(0xFF44), 144);
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x01, 0x01); // VBlank requested (IF bit 0)
    }

    #[test]
    fn test_ppu_ly_wraps_after_153() {
        let mut cpu = setup_cpu(vec![]);
        // a full frame is 154 scanlines; after it LY should be back at 0
        cpu.step_ppu(456 * 154);

        assert_eq!(cpu.memory.read_byte(0xFF44), 0);
    }

    #[test]
    fn test_ppu_mode_progreses_with_scanline() {
        let mut cpu = setup_cpu(vec![]);
        cpu.step_ppu(4); // dot 4 -> OAM scan
        assert_eq!(cpu.memory.read_byte(0xFF41) & 0b11, 2);

        cpu.step_ppu(100); // dot 104 -> Drawing
        assert_eq!(cpu.memory.read_byte(0xFF41) & 0b11, 3);

        cpu.step_ppu(200); // dot 304 -> HBlank
        assert_eq!(cpu.memory.read_byte(0xFF41) & 0b11, 0);
    }

    #[test]
    fn test_stat_preserves_enable_bits_and_sets_coincidence() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF45, 0x00); // LYC = 0 (matches starting LY)
        cpu.memory.write_byte(0xFF41, 0b0111_1000); // game sets all 4 enable bits
        cpu.step_ppu(4);

        let stat = cpu.memory.read_byte(0xFF41);
        assert_eq!(stat & 0b0111_1000, 0b0111_1000); // the game's enable bits (3-6) survived the
        // PPU's status write

        assert_eq!(stat & 0x04, 0x04); // LYC == LY, so the coincidence flag (bit 2) is set
    }

    #[test]
    fn test_ppu_mode_is_blank_on_line_144() {
        let mut cpu = setup_cpu(vec![]);
        cpu.step_ppu(456 * 144); // LY = 144 -> VBlank
        assert_eq!(cpu.memory.read_byte(0xFF41) & 0b11, 1);
    }

    #[test]
    fn test_stat_interrupt_fires_on_hblank() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF41, 0x08); // arm Mode 0 (HBlank) STAT interrupt
        cpu.step_ppu(4); // Mode 2 ( not HBlank, no fire)
        cpu.step_ppu(300); // dot 304 -> enters mode 0
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x02, 0x02); // IF bit 1 set
    }

    #[test]
    fn test_stat_interrupt_silent_when_disabled() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_word(0xFF41, 0x00); // no enabled armed
        cpu.step_ppu(4);
        cpu.step_ppu(300); // enters Mode 0, but disabled
        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x02, 0x00); // no STAT interrupt
    }

    #[test]
    fn teset_stat_interrupt_fires_on_lyc_coincidence() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF45, 0x01); // LYC = 1
        cpu.memory.write_byte(0xFF41, 0x40); // arm LYC interrupt
        cpu.step_ppu(456); // LY -> 1, coincidence rises

        assert_eq!(cpu.memory.read_byte(0xFF0F) & 0x02, 0x02);
    }

    #[test]
    fn test_decode_tile_row() {
        assert_eq!(decode_tile_row(0x3C, 0x7E), [0, 2, 3, 3, 3, 3, 2, 0]);
    }

    #[test]
    fn test_decode_tile_row_solid_colors() {
        assert_eq!(decode_tile_row(0xFF, 0xFF), [3; 8]); // both bits set -> all 3 
        assert_eq!(decode_tile_row(0xFF, 0x00), [1; 8]); // only low -> all 1
        assert_eq!(decode_tile_row(0x00, 0xFF), [2; 8]); // only high -> all 2
    }

    #[test]
    fn test_render_bg_scanline() {
        let mut cpu = setup_cpu(vec![]);
        // tile #1, row 0: low=0x3C high=0x7E -> [0, 2, 3, 3, 3, 3, 2, 0]
        cpu.memory.write_byte(0x8010, 0x3C); // 0x8000 + 1*16
        cpu.memory.write_byte(0x8011, 0x7E);
        cpu.memory.write_byte(0x9800, 0x01); // map (0,0) -> tile #1
        cpu.memory.write_byte(0xFF42, 0); // SCY = 0
        cpu.memory.write_byte(0xFF43, 0); // SCX = 0
        cpu.memory.write_byte(0xFF40, 0b0001_0001); // LCDC: unsigned 0x8000 data + BG enable
        // data
        cpu.memory.write_byte(0xFF47, 0xE4); // BGP = identity (id n -> shade n)

        cpu.render_scanline(0);

        assert_eq!(&cpu.framebuffer[0..8], &[0u8, 2, 3, 3, 3, 3, 2, 0].map(dmg_rgb));
    }

    #[test]
    fn test_apply_palette() {
        let identity = 0xE4; // 11_10_01_00 : id n -> shade n
        assert_eq!(apply_palette(identity, 1), 1);
        assert_eq!(apply_palette(identity, 3), 3);

        let inverted = 0x1B; // 00_01_10_11 : id0 -> 3, id3 -> 0
        assert_eq!(apply_palette(inverted, 0), 3);
        assert_eq!(apply_palette(inverted, 3), 0);
    }

    #[test]
    fn test_render_signed_tile_data() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0x8FF0, 0x3c); // tile -1, row 0 (0x9000 -16)
        cpu.memory.write_byte(0x8FF1, 0x7E);
        cpu.memory.write_byte(0x9800, 0xFF); // map (0,0) -> tile index 0xFF (-1)
        cpu.memory.write_byte(0xFF40, 0x01); // bit4=0 signed 0x9000 mode + BG enable
        cpu.memory.write_byte(0xFF47, 0xE4); // identity palette
        cpu.render_scanline(0);
        assert_eq!(&cpu.framebuffer[0..8], &[0u8, 2, 3, 3, 3, 3, 2, 0].map(dmg_rgb));
    }

    #[test]
    fn test_ppu_renders_line_via_step() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0x8010, 0x3C); // tile #1 row 0 
        cpu.memory.write_byte(0x8011, 0x7E);
        cpu.memory.write_byte(0x9800, 0x01); // map (0,0) -> tile #1
        cpu.memory.write_byte(0xFF40, 0b1001_0001); // LCDC: LCD on + unsigned data + BG enable
        cpu.memory.write_byte(0xFF47, 0xE4); // identity palette

        // Lines render at their START. Start at LY 153 so completing it rolls into
        // line 0, which then renders.
        cpu.memory.write_byte(0xFF44, 153);
        cpu.step_ppu(456);

        assert_eq!(&cpu.framebuffer[0..8], &[0u8, 2, 3, 3, 3, 3, 2, 0].map(dmg_rgb));
    }
    #[test]
    fn test_ppu_lcd_disabled_holds_ly_zero() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0xFF40, 0x00); // LCD off (bit 7 clear)
        cpu.step_ppu(456 * 5); // would normally advance LY several lines
        assert_eq!(cpu.memory.read_byte(0xFF44), 0); // LY pinned at 0
    }

    #[test]
    fn test_ppu_bg_disabled_renders_blank() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0x8010, 0x3C); // a real tile is present...
        cpu.memory.write_byte(0x8011, 0x7E);
        cpu.memory.write_byte(0x9800, 0x01);
        cpu.memory.write_byte(0xFF47, 0xE4);
        cpu.memory.write_byte(0xFF40, 0b1001_0000); // LCD on (bit7), data 0x8000 (bit4), BG OFF
        // (bit0=0)
        cpu.render_scanline(0);
        assert_eq!(&cpu.framebuffer[0..8], &[dmg_rgb(0); 8]); // ...but BG-disable blanks it
    }

    #[test]
    fn test_debug_state_reports_initial_state() {
        let cpu = Cpu::new();
        let s = cpu.debug_state();
        // Fresh CPU starts at the cartridge entry point with the stack at the top.
        assert!(s.contains("PC:0100"), "debug_state was: {s}");
        assert!(s.contains("SP:FFFE"), "debug_state was: {s}");
        // Post-boot LCDC default we seed in Memory::new.
        assert!(s.contains("LCDC:91"), "debug_state was: {s}");
    }

    #[test]
    fn test_sprite_ten_per_line_limit() {
        let mut cpu = setup_cpu(vec![]);
        cpu.memory.write_byte(0x8010, 0xFF); // tile 1 = solid color 3
        cpu.memory.write_byte(0x8011, 0xFF);
        cpu.memory.write_byte(0xFF48, 0xE4);
        cpu.memory.write_byte(0xFF40, 0b1001_0010); // LCD + unsigned data + sprites

        // 11 non-overlapping sprites across line 0 (screen x = 0, 8, ... 80).
        for n in 0..11u16 {
            let base = 0xFE00 + n * 4;
            cpu.memory.write_byte(base, 16); // Y -> line 0
            cpu.memory.write_byte(base + 1, 8 + (n as u8) * 8); // X
            cpu.memory.write_byte(base + 2, 1);
            cpu.memory.write_byte(base + 3, 0);
        }
        cpu.render_sprites(0);

        let blank = (224, 248, 208);
        assert_ne!(cpu.framebuffer[0], blank); // sprite 0 drawn
        assert_eq!(cpu.framebuffer[80], blank); // 11th sprite dropped (over 10-limit)
    }

    #[test]
    fn test_sprite_behind_bg_priority() {
        let mut cpu = setup_cpu(vec![]);
        // BG tile 1 = solid color 1 (low=0xFF, high=0x00).
        cpu.memory.write_byte(0x8010, 0xFF);
        cpu.memory.write_byte(0x8011, 0x00);
        cpu.memory.write_byte(0x9800, 0x01);
        // Sprite tile 2 = solid color 3, placed at screen (0,0), flagged behind BG.
        cpu.memory.write_byte(0x8020, 0xFF);
        cpu.memory.write_byte(0x8021, 0xFF);
        cpu.memory.write_byte(0xFE00, 16); // Y -> screen 0
        cpu.memory.write_byte(0xFE01, 8); // X -> screen 0
        cpu.memory.write_byte(0xFE02, 2); // tile 2
        cpu.memory.write_byte(0xFE03, 0x80); // behind-BG priority
        cpu.memory.write_byte(0xFF47, 0xE4);
        cpu.memory.write_byte(0xFF48, 0xE4);
        cpu.memory.write_byte(0xFF40, 0b1001_0011); // LCD + unsigned data + sprites + BG on

        cpu.render_scanline(0);
        let bg_pixel = cpu.framebuffer[0]; // BG color 1
        cpu.render_sprites(0);
        // BG is opaque and the sprite is behind it -> the BG pixel is preserved.
        assert_eq!(cpu.framebuffer[0], bg_pixel);

        // Now clear the behind-BG flag: the sprite (color 3) should win.
        cpu.memory.write_byte(0xFE03, 0x00);
        cpu.render_scanline(0);
        cpu.render_sprites(0);
        assert_ne!(cpu.framebuffer[0], bg_pixel);
    }

    #[test]
    fn test_stop_performs_speed_switch() {
        let mut cpu = setup_cpu(vec![0x10, 0x00]); // STOP
        cpu.memory.write_byte(0xFF4D, 0x01); // arm the speed switch
        cpu.step();
        assert!(cpu.memory.double_speed); // switched
        assert!(!cpu.halted); // a speed switch resumes execution, not a real stop
    }

    #[test]
    fn test_render_window_layer() {
        let mut cpu = setup_cpu(vec![]);
        // tile #1, curve pattern, in the 0x8000 (unsigned) region
        cpu.memory.write_byte(0x8010, 0x3C);
        cpu.memory.write_byte(0x8011, 0x7E);
        cpu.memory.write_byte(0x9C00, 0x01); // WINDOW map (0,0) -> tile #1
        // BG map at 0x9800 left all-zero (blank) so only the window can produce this.
        cpu.memory.write_byte(0xFF47, 0xE4); // identity palette
        cpu.memory.write_byte(0xFF4A, 0x00); // WY = 0
        cpu.memory.write_byte(0xFF4B, 0x07); // WX = 7 -> window starts at screen x=0
        // LCDC: LCD on, window map 0x9C00 (bit6), window on (bit5), unsigned data (bit4), BG on (bit0)
        cpu.memory.write_byte(0xFF40, 0b1111_0001);

        cpu.render_scanline(0);

        assert_eq!(&cpu.framebuffer[0..8], &[0u8, 2, 3, 3, 3, 3, 2, 0].map(dmg_rgb));
    }

    #[test]
    fn test_render_sprite_with_transparency() {
        let mut cpu = setup_cpu(vec![]);
        // sprite tile #1, curve pattern row 0 -> color ids [0,2,3,3,3,3,2,0]
        cpu.memory.write_byte(0x8010, 0x3C);
        cpu.memory.write_byte(0x8011, 0x7E);
        // OAM sprite 0: Y=16 (screen y=0), X=8 (screen x=0), tile 1, no flags
        cpu.memory.write_byte(0xFE00, 16);
        cpu.memory.write_byte(0xFE01, 8);
        cpu.memory.write_byte(0xFE02, 1);
        cpu.memory.write_byte(0xFE03, 0);
        cpu.memory.write_byte(0xFF48, 0xE4); // OBP0 identity
        cpu.memory.write_byte(0xFF40, 0b1000_0010); // LCD on (bit7), sprites on (bit1)

        cpu.render_sprites(0);

        // Color 0 is transparent, so px 0 and 7 stay at the framebuffer's 0; the
        // rest are the sprite's shades.
        assert_eq!(&cpu.framebuffer[0..8], &[0u8, 2, 3, 3, 3, 3, 2, 0].map(dmg_rgb));
    }
}
