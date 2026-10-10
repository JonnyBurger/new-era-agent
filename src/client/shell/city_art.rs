//! Small, deterministic pixel buildings for the client-owned city overview.
//!
//! Two vertical pixels share one terminal cell. Sampling straight into the
//! destination buffer keeps this render path independent of terminals and avoids
//! allocating a raster for each agent. Colours follow the supplied Pixel Zurich
//! roof reference; all geometry is native terminal drawing.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

const OUTLINE: Color = Color::Rgb(38, 34, 43);
const GRASS: Color = Color::Rgb(134, 191, 99);
const SHADOW: Color = Color::Rgb(83, 126, 67);
const ROOFS: [[Color; 6]; 6] = [
    palette([0xf4a47c, 0xde7a52, 0xc55f3f, 0xa14732, 0x7c3427, 0x55231d]),
    palette([0xecc79a, 0xd3a26f, 0xbb8654, 0x976743, 0x734b32, 0x4f3324]),
    palette([0xb9c5d4, 0x8f9eb3, 0x727f96, 0x59647c, 0x434c61, 0x2e3445]),
    palette([0x9a9da6, 0x7b7f89, 0x62666f, 0x4d5059, 0x3a3c44, 0x28292f]),
    palette([0xb2e6d2, 0x82c9b1, 0x5fab94, 0x47897c, 0x336a61, 0x224a45]),
    palette([0xe0ddd2, 0xc8c5b9, 0xb0ad9f, 0x959284, 0x79766a, 0x5a584f]),
];

const fn rgb(value: u32) -> Color {
    Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

const fn palette(values: [u32; 6]) -> [Color; 6] {
    [
        rgb(values[0]),
        rgb(values[1]),
        rgb(values[2]),
        rgb(values[3]),
        rgb(values[4]),
        rgb(values[5]),
    ]
}

/// Paint a transparent pixel layer, preserving either half of uncovered cells.
fn paint(buffer: &mut Buffer, area: Rect, sample: impl Fn(i32, i32) -> Option<Color>) {
    let clipped = area.intersection(buffer.area);
    for y in clipped.y..clipped.bottom() {
        let py = i32::from(y - area.y) * 2;
        for x in clipped.x..clipped.right() {
            let px = i32::from(x - area.x);
            let upper = sample(px, py);
            let lower = sample(px, py + 1);
            if upper.is_none() && lower.is_none() {
                continue;
            }
            if let Some(cell) = buffer.cell_mut((x, y)) {
                let old_upper = if cell.symbol() == "▀" {
                    cell.fg
                } else {
                    cell.bg
                };
                cell.set_char('▀');
                cell.fg = upper.unwrap_or(old_upper);
                cell.bg = lower.unwrap_or(cell.bg);
                cell.modifier = Modifier::empty();
            }
        }
    }
}

fn noise(x: i32, y: i32, seed: u64) -> u64 {
    let value = seed
        .wrapping_add((x as u64).wrapping_mul(0x9e37_79b9))
        .wrapping_add((y as u64).wrapping_mul(0x85eb_ca6b));
    (value ^ (value >> 16)).wrapping_mul(0xc2b2_ae35)
}

pub(super) fn draw_ground(buffer: &mut Buffer, area: Rect) {
    paint(buffer, area, |x, y| {
        // Absolute coordinates keep the lawn still when individual lots redraw.
        let grain = noise(x + i32::from(area.x), y + i32::from(area.y) * 2, 7) % 47;
        Some(match grain {
            0..=2 => rgb(0x73ab53),
            3 => rgb(0x9fd17a),
            _ => GRASS,
        })
    });
}

/// A paved boulevard with a pair of silver tram rails and pale stone kerbs.
pub(super) fn draw_street(buffer: &mut Buffer, area: Rect) {
    let height = i32::from(area.height) * 2;
    paint(buffer, area, |x, y| {
        let x = x + i32::from(area.x);
        Some(if y == 0 || y == height - 1 {
            if x % 6 == 0 {
                rgb(0xb2b1a7)
            } else {
                rgb(0xd6d3c5)
            }
        } else if y == 1 || y == height - 2 {
            rgb(0xf2f0ea)
        } else if height >= 8 && (y == height / 2 - 1 || y == height / 2 + 1) {
            rgb(0xc9ccd4)
        } else if height >= 8 && y == height / 2 {
            rgb(0x3b3d45)
        } else if x % 7 == 0 || noise(x, y, 11).is_multiple_of(13) {
            rgb(0x50525b)
        } else {
            rgb(0x5c5f68)
        })
    });
}

pub(super) fn draw_tree(buffer: &mut Buffer, area: Rect, seed: u64) {
    let width = i32::from(area.width);
    let height = i32::from(area.height) * 2;
    let radius = ((width.min(height) - 2) / 2).max(1);
    let cx = (width - 1) / 2;
    let cy = (height - 2) / 2;
    paint(buffer, area, |x, y| {
        let dx = x - cx;
        let dy = y - cy;
        let distance = dx * dx + dy * dy;
        if distance <= radius * radius {
            Some(if distance > (radius - 1) * (radius - 1) {
                rgb(0x36652a)
            } else if dx + dy < -radius / 2 {
                rgb(0xc4e88f)
            } else if dx + dy > radius / 2 {
                rgb(0x4d8a33)
            } else if noise(x, y, seed).is_multiple_of(5) {
                rgb(0x6aad43)
            } else {
                rgb(0x8fcc5c)
            })
        } else if dx == 0 && dy > 0 && dy <= radius + 1 {
            Some(rgb(0x734b32))
        } else if (dx - 1) * (dx - 1) + (dy - 1) * (dy - 1) <= radius * radius {
            Some(SHADOW)
        } else {
            None
        }
    });
}

/// Render one agent's house; identity, status and selection are supplied by the
/// caller, so no runtime access or presentation-state mutation happens here.
pub(super) fn draw_house(
    buffer: &mut Buffer,
    area: Rect,
    seed: u64,
    accent: Color,
    selected: bool,
) {
    let width = i32::from(area.width);
    let height = i32::from(area.height) * 2;
    if width < 8 || height < 8 {
        return;
    }

    let left = 2;
    let right = width - 3;
    let top = 2;
    let bottom = height - 3;
    let wall_height = (height / 4).clamp(2, 4);
    let eave = bottom - wall_height;
    let roof_width = right - left + 1;
    let roof_height = eave - top + 1;
    let material = ((seed / 5) % ROOFS.len() as u64) as usize;
    let roof_kind = seed % 5;
    let colours = &ROOFS[material];
    let chimney_x = left + (roof_width * 2 / 3).min(roof_width - 3);
    let door_x = left + roof_width / 2;

    paint(buffer, area, |x, y| {
        // A small chimney breaks up the roof silhouette without increasing the
        // lot's extent. Flat roofs use a light stone ventilation stack instead.
        if x >= chimney_x && x <= chimney_x + 1 && y >= top - 1 && y <= top + 1 {
            return Some(if y == top - 1 {
                OUTLINE
            } else if roof_kind == 4 {
                if x == chimney_x {
                    rgb(0xe0ddd2)
                } else {
                    rgb(0x959284)
                }
            } else if x == chimney_x {
                rgb(0xd98a63)
            } else {
                rgb(0x7c3427)
            });
        }

        let inside = x >= left && x <= right && y >= top && y <= bottom;
        if !inside {
            if selected && x >= left - 1 && x <= right + 1 && y >= top - 1 && y <= bottom + 1 {
                return Some(accent);
            }
            if x > left && x <= right + 2 && y >= top + 2 && y <= bottom + 2 {
                return Some(SHADOW);
            }
            return None;
        }

        if x == left || x == right || y == top || y == bottom || y == eave {
            return Some(OUTLINE);
        }
        if y < eave {
            let rx = x - left;
            let ry = y - top;
            // A tiny blue skylight, with its sunlit sill on the lower edge.
            if seed % 2 == 1
                && (3..=4).contains(&rx)
                && ry >= roof_height - 4
                && ry <= roof_height - 3
            {
                return Some(if ry == roof_height - 3 {
                    colours[0]
                } else if rx == 3 {
                    rgb(0xa9d7dc)
                } else {
                    rgb(0x4e788b)
                });
            }
            return Some(roof_pixel(
                rx,
                ry,
                roof_width,
                roof_height,
                roof_kind,
                material,
                colours,
            ));
        }

        let wall_y = y - eave;
        if x == door_x && y >= bottom - 2 {
            return Some(if y == bottom - 2 {
                accent
            } else {
                rgb(0x635149)
            });
        }
        if wall_y <= 2 && (x - left) % 4 == 2 {
            return Some(if wall_y == 1 {
                rgb(0x514b51)
            } else {
                rgb(0xffdc8a)
            });
        }
        Some(if x >= right - 2 {
            rgb(0xb3a290)
        } else if material == 2 || material == 4 {
            rgb(0xe4deca)
        } else {
            rgb(0xf0d3a8)
        })
    });
}

fn roof_pixel(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    kind: u64,
    material: usize,
    colours: &[Color; 6],
) -> Color {
    let north = y;
    let south = height - 1 - y;
    let west = x;
    let east = width - 1 - x;
    let edge = north.min(south).min(west).min(east);
    let (shade, slope, along, ridge) = match kind {
        0 => (
            if north < south { 1 } else { 3 },
            north.min(south),
            x,
            (north - south).abs() <= 1,
        ),
        1 => (
            if west < east { 1 } else { 3 },
            west.min(east),
            y,
            (west - east).abs() <= 1,
        ),
        4 => {
            return if edge == 1 {
                colours[0]
            } else if north == 2 || west == 2 {
                colours[4]
            } else {
                colours[2 + usize::from((x + y * 3) % 7 == 0)]
            };
        }
        3 if edge > 2 => return colours[if edge == 3 { 0 } else { 2 }],
        _ => {
            let horizontal = north.min(south);
            let vertical = west.min(east);
            let shade = if north == edge {
                1
            } else if west == edge {
                2
            } else {
                3
            };
            (
                shade,
                edge,
                if horizontal <= vertical { x } else { y },
                horizontal == vertical,
            )
        }
    };
    if ridge {
        return colours[0];
    }
    let texture = if material == 4 {
        -i32::from(along % 4 == 0)
    } else if material == 2 || material == 3 {
        i32::from(slope % 2 == 0 || (along + slope / 2 * 3) % 6 == 0)
    } else {
        i32::from(slope % 3 == 0 || (along + slope / 3 * 2) % 4 == 0)
    };
    colours[(shade + texture).clamp(0, 5) as usize]
}
