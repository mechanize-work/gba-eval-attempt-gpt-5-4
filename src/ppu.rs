use crate::{SCREEN_HEIGHT, SCREEN_WIDTH};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Layer {
    Backdrop,
    Bg(u8),
    Obj,
}

#[derive(Clone, Copy)]
struct Pixel {
    color: u16,
    priority: u8,
    order: u8,
    layer: Layer,
    semi_transparent: bool,
}

pub(crate) fn render_framebuffer(
    io: &[u8],
    palette: &[u8],
    vram: &[u8],
    oam: &[u8],
    out: &mut [u32],
) {
    let dispcnt = read_u16(io, 0x000);
    if dispcnt & (1 << 7) != 0 {
        out.fill(0xffff_ffff);
        return;
    }
    let mode = dispcnt & 0x7;
    let obj_enabled = dispcnt & (1 << 12) != 0;

    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            let backdrop = Pixel {
                color: read_palette_color(palette, 0),
                priority: 4,
                order: 0xff,
                layer: Layer::Backdrop,
                semi_transparent: false,
            };
            let mut top = [backdrop; 2];

            match mode {
                0 => {
                    for bg in 0..4 {
                        if dispcnt & (1 << (8 + bg)) == 0 {
                            continue;
                        }
                        if let Some(pixel) = render_regular_bg_pixel(bg, x as i32, y as i32, io, palette, vram) {
                            top = insert_pixel(top, pixel);
                        }
                    }
                }
                1 => {
                    for bg in 0..2 {
                        if dispcnt & (1 << (8 + bg)) == 0 {
                            continue;
                        }
                        if let Some(pixel) = render_regular_bg_pixel(bg, x as i32, y as i32, io, palette, vram) {
                            top = insert_pixel(top, pixel);
                        }
                    }
                }
                3 => {
                    if dispcnt & (1 << 10) != 0 {
                        if let Some(pixel) = render_mode3_pixel(x, y, io, vram) {
                            top = insert_pixel(top, pixel);
                        }
                    }
                }
                4 => {
                    if dispcnt & (1 << 10) != 0 {
                        if let Some(pixel) = render_mode4_pixel(x, y, io, palette, vram, dispcnt) {
                            top = insert_pixel(top, pixel);
                        }
                    }
                }
                5 => {
                    if dispcnt & (1 << 10) != 0 {
                        if let Some(pixel) = render_mode5_pixel(x, y, io, vram, dispcnt) {
                            top = insert_pixel(top, pixel);
                        }
                    }
                }
                _ => {}
            }

            if obj_enabled {
                if let Some(pixel) = render_obj_pixel(x as i32, y as i32, dispcnt, palette, vram, oam) {
                    top = insert_pixel(top, pixel);
                }
            }

            out[y * SCREEN_WIDTH + x] = bgr555_to_abgr(apply_color_effect(io, top[0], top[1]));
        }
    }
}

fn pixel_before(lhs: Pixel, rhs: Pixel) -> bool {
    lhs.priority < rhs.priority || (lhs.priority == rhs.priority && lhs.order < rhs.order)
}

fn insert_pixel(mut top: [Pixel; 2], candidate: Pixel) -> [Pixel; 2] {
    if pixel_before(candidate, top[0]) {
        top[1] = top[0];
        top[0] = candidate;
    } else if pixel_before(candidate, top[1]) {
        top[1] = candidate;
    }
    top
}

fn apply_color_effect(io: &[u8], primary: Pixel, secondary: Pixel) -> u16 {
    let bldcnt = read_u16(io, 0x050);
    let effect = ((bldcnt >> 6) & 0x3) as u8;
    let first_targets = bldcnt & 0x003f;
    let second_targets = (bldcnt >> 8) & 0x003f;
    let primary_mask = layer_mask(primary.layer);
    let secondary_mask = layer_mask(secondary.layer);
    let alpha_enabled = primary.semi_transparent || effect == 1;

    if alpha_enabled
        && (primary.semi_transparent || primary_mask & first_targets != 0)
        && secondary_mask & second_targets != 0
    {
        let bldalpha = read_u16(io, 0x052);
        let eva = ((bldalpha & 0x1f).min(16)) as u8;
        let evb = (((bldalpha >> 8) & 0x1f).min(16)) as u8;
        return alpha_blend(primary.color, secondary.color, eva, evb);
    }

    if primary_mask & first_targets == 0 || primary.semi_transparent {
        return primary.color;
    }

    let evy = ((read_u16(io, 0x054) & 0x1f).min(16)) as u8;
    match effect {
        2 => brighten(primary.color, evy),
        3 => darken(primary.color, evy),
        _ => primary.color,
    }
}

fn layer_mask(layer: Layer) -> u16 {
    match layer {
        Layer::Bg(0) => 1 << 0,
        Layer::Bg(1) => 1 << 1,
        Layer::Bg(2) => 1 << 2,
        Layer::Bg(3) => 1 << 3,
        Layer::Obj => 1 << 4,
        Layer::Backdrop => 1 << 5,
        Layer::Bg(_) => 0,
    }
}

fn alpha_blend(primary: u16, secondary: u16, eva: u8, evb: u8) -> u16 {
    blend_channels(primary, secondary, eva, evb, |a, b, x, y| ((a * x + b * y) >> 4).min(31))
}

fn brighten(color: u16, evy: u8) -> u16 {
    brighten_or_darken(color, evy, |channel, y| channel + (((31 - channel) * y) >> 4))
}

fn darken(color: u16, evy: u8) -> u16 {
    brighten_or_darken(color, evy, |channel, y| channel - ((channel * y) >> 4))
}

fn blend_channels<F>(primary: u16, secondary: u16, eva: u8, evb: u8, mut f: F) -> u16
where
    F: FnMut(u16, u16, u16, u16) -> u16,
{
    let eva = eva as u16;
    let evb = evb as u16;
    let r = f(primary & 0x1f, secondary & 0x1f, eva, evb);
    let g = f((primary >> 5) & 0x1f, (secondary >> 5) & 0x1f, eva, evb);
    let b = f((primary >> 10) & 0x1f, (secondary >> 10) & 0x1f, eva, evb);
    r | (g << 5) | (b << 10)
}

fn brighten_or_darken<F>(color: u16, evy: u8, mut f: F) -> u16
where
    F: FnMut(u16, u16) -> u16,
{
    let evy = evy as u16;
    let r = f(color & 0x1f, evy).min(31);
    let g = f((color >> 5) & 0x1f, evy).min(31);
    let b = f((color >> 10) & 0x1f, evy).min(31);
    r | (g << 5) | (b << 10)
}

fn render_mode3_pixel(x: usize, y: usize, io: &[u8], vram: &[u8]) -> Option<Pixel> {
    let idx = (y * SCREEN_WIDTH + x) * 2;
    let color = read_u16(vram, idx);
    let priority = (read_u16(io, 0x00c) & 0x3) as u8;
    Some(Pixel {
        color,
        priority,
        order: 4,
        layer: Layer::Bg(2),
        semi_transparent: false,
    })
}

fn render_mode4_pixel(x: usize, y: usize, io: &[u8], palette: &[u8], vram: &[u8], dispcnt: u16) -> Option<Pixel> {
    let page = if dispcnt & (1 << 4) != 0 { 0xa000 } else { 0 };
    let idx = page + y * SCREEN_WIDTH + x;
    let palette_index = vram.get(idx).copied().unwrap_or(0) as usize;
    let color = read_palette_color(palette, palette_index as u16);
    let priority = (read_u16(io, 0x00c) & 0x3) as u8;
    Some(Pixel {
        color,
        priority,
        order: 4,
        layer: Layer::Bg(2),
        semi_transparent: false,
    })
}

fn render_mode5_pixel(x: usize, y: usize, io: &[u8], vram: &[u8], dispcnt: u16) -> Option<Pixel> {
    if x >= 160 || y >= 128 {
        return None;
    }
    let page = if dispcnt & (1 << 4) != 0 { 0xa000 } else { 0 };
    let idx = page + (y * 160 + x) * 2;
    let color = read_u16(vram, idx);
    let priority = (read_u16(io, 0x00c) & 0x3) as u8;
    Some(Pixel {
        color,
        priority,
        order: 4,
        layer: Layer::Bg(2),
        semi_transparent: false,
    })
}

fn render_regular_bg_pixel(
    bg: usize,
    x: i32,
    y: i32,
    io: &[u8],
    palette: &[u8],
    vram: &[u8],
) -> Option<Pixel> {
    let control = read_u16(io, 0x008 + bg * 2);
    let priority = (control & 0x3) as u8;
    let char_base = ((control >> 2) & 0x3) as usize * 0x4000;
    let is_8bpp = control & (1 << 7) != 0;
    let screen_base = ((control >> 8) & 0x1f) as usize * 0x800;
    let size = (control >> 14) & 0x3;

    let (width, height, blocks_per_row) = match size {
        0 => (256, 256, 1),
        1 => (512, 256, 2),
        2 => (256, 512, 1),
        _ => (512, 512, 2),
    };

    let hofs = (read_u16(io, 0x010 + bg * 4) & 0x1ff) as i32;
    let vofs = (read_u16(io, 0x012 + bg * 4) & 0x1ff) as i32;
    let sx = (x + hofs).rem_euclid(width as i32) as usize;
    let sy = (y + vofs).rem_euclid(height as i32) as usize;
    let screen_x = sx / 256;
    let screen_y = sy / 256;
    let local_x = sx % 256;
    let local_y = sy % 256;
    let screenblock = screen_base + (screen_x + screen_y * blocks_per_row) * 0x800;
    let tile_x = local_x / 8;
    let tile_y = local_y / 8;
    let entry_addr = screenblock + (tile_y * 32 + tile_x) * 2;
    let entry = read_u16(vram, entry_addr);
    let mut px = local_x & 7;
    let mut py = local_y & 7;
    if entry & (1 << 10) != 0 {
        px = 7 - px;
    }
    if entry & (1 << 11) != 0 {
        py = 7 - py;
    }

    let tile_index = (entry & 0x03ff) as usize;
    let palette_bank = ((entry >> 12) & 0xf) as usize;
    let color_index = if is_8bpp {
        let tile_addr = char_base + tile_index * 64 + py * 8 + px;
        vram.get(tile_addr).copied().unwrap_or(0) as usize
    } else {
        let tile_addr = char_base + tile_index * 32 + py * 4 + px / 2;
        let byte = vram.get(tile_addr).copied().unwrap_or(0);
        let nibble = if px & 1 == 0 { byte & 0x0f } else { byte >> 4 };
        nibble as usize
    };

    if color_index == 0 {
        return None;
    }

    let palette_index = if is_8bpp {
        color_index as u16
    } else {
        (palette_bank * 16 + color_index) as u16
    };
    Some(Pixel {
        color: read_palette_color(palette, palette_index),
        priority,
        order: 1 + bg as u8,
        layer: Layer::Bg(bg as u8),
        semi_transparent: false,
    })
}

fn render_obj_pixel(
    x: i32,
    y: i32,
    dispcnt: u16,
    palette: &[u8],
    vram: &[u8],
    oam: &[u8],
) -> Option<Pixel> {
    let one_d = dispcnt & (1 << 6) != 0;
    let mut best: Option<Pixel> = None;

    for i in (0..128).rev() {
        let base = i * 8;
        let attr0 = read_u16(oam, base);
        let attr1 = read_u16(oam, base + 2);
        let attr2 = read_u16(oam, base + 4);

        let shape = ((attr0 >> 14) & 0x3) as usize;
        let size = ((attr1 >> 14) & 0x3) as usize;
        let Some((width, height)) = sprite_dimensions(shape, size) else {
            continue;
        };

        let affine = attr0 & (1 << 8) != 0;
        let double_or_disable = attr0 & (1 << 9) != 0;
        let obj_mode = (attr0 >> 10) & 0x3;
        if !affine && double_or_disable {
            continue;
        }
        if affine {
            continue;
        }
        if obj_mode == 2 {
            continue;
        }

        let color_256 = attr0 & (1 << 13) != 0;
        let x_pos = wrap_coord(attr1 & 0x01ff, 512);
        let y_pos = wrap_coord(attr0 & 0x00ff, 256);
        if x < x_pos || x >= x_pos + width as i32 || y < y_pos || y >= y_pos + height as i32 {
            continue;
        }

        let mut px = (x - x_pos) as usize;
        let mut py = (y - y_pos) as usize;
        if attr1 & (1 << 12) != 0 {
            px = width - 1 - px;
        }
        if attr1 & (1 << 13) != 0 {
            py = height - 1 - py;
        }

        let tile_index = (attr2 & 0x03ff) as usize;
        let priority = ((attr2 >> 10) & 0x3) as u8;
        let palette_bank = ((attr2 >> 12) & 0xf) as usize;
        let tile_x = px / 8;
        let tile_y = py / 8;
        let in_tile_x = px & 7;
        let in_tile_y = py & 7;
        let tile_number = if color_256 {
            let stride = if one_d { width / 4 } else { 32 };
            (tile_index & !1) + tile_y * stride + tile_x * 2
        } else {
            let stride = if one_d { width / 8 } else { 32 };
            tile_index + tile_y * stride + tile_x
        };
        let tile_addr = 0x10000 + tile_number * 32;
        let color_index = if color_256 {
            vram.get(tile_addr + in_tile_y * 8 + in_tile_x).copied().unwrap_or(0) as usize
        } else {
            let byte = vram
                .get(tile_addr + in_tile_y * 4 + in_tile_x / 2)
                .copied()
                .unwrap_or(0);
            let nibble = if in_tile_x & 1 == 0 { byte & 0x0f } else { byte >> 4 };
            nibble as usize
        };
        if color_index == 0 {
            continue;
        }

        let palette_index = if color_256 {
            0x100 + color_index as u16
        } else {
            0x100 + (palette_bank * 16 + color_index) as u16
        };
        let pixel = Pixel {
            color: read_palette_color(palette, palette_index),
            priority,
            order: 0,
            layer: Layer::Obj,
            semi_transparent: obj_mode == 1,
        };
        best = Some(match best {
            Some(current) => {
                if pixel_before(pixel, current) {
                    pixel
                } else {
                    current
                }
            }
            None => pixel,
        });
    }

    best
}

fn sprite_dimensions(shape: usize, size: usize) -> Option<(usize, usize)> {
    match (shape, size) {
        (0, 0) => Some((8, 8)),
        (0, 1) => Some((16, 16)),
        (0, 2) => Some((32, 32)),
        (0, 3) => Some((64, 64)),
        (1, 0) => Some((16, 8)),
        (1, 1) => Some((32, 8)),
        (1, 2) => Some((32, 16)),
        (1, 3) => Some((64, 32)),
        (2, 0) => Some((8, 16)),
        (2, 1) => Some((8, 32)),
        (2, 2) => Some((16, 32)),
        (2, 3) => Some((32, 64)),
        _ => None,
    }
}

fn wrap_coord(raw: u16, range: i32) -> i32 {
    let value = raw as i32;
    if value >= range / 2 {
        value - range
    } else {
        value
    }
}

fn read_palette_color(palette: &[u8], index: u16) -> u16 {
    read_u16(palette, index as usize * 2)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    let lo = bytes.get(offset).copied().unwrap_or(0) as u16;
    let hi = bytes.get(offset + 1).copied().unwrap_or(0) as u16;
    lo | (hi << 8)
}

fn bgr555_to_abgr(color: u16) -> u32 {
    let r5 = (color & 0x1f) as u32;
    let g5 = ((color >> 5) & 0x1f) as u32;
    let b5 = ((color >> 10) & 0x1f) as u32;
    let r = (r5 << 3) | (r5 >> 2);
    let g = (g5 << 3) | (g5 >> 2);
    let b = (b5 << 3) | (b5 >> 2);
    0xff00_0000 | (b << 16) | (g << 8) | r
}

#[cfg(test)]
mod tests {
    use super::alpha_blend;

    #[test]
    fn alpha_blend_can_fully_select_the_secondary_color() {
        let primary = 0x001f;
        let secondary = 0x7c00;
        assert_eq!(alpha_blend(primary, secondary, 0, 16), secondary);
    }
}
