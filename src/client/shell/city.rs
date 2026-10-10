//! Client-only city geometry and drawing. No terminal/runtime state is touched here.

use super::*;
use crate::api::schema::AgentStatus;
use ratatui::style::Color;

const HEADER: u16 = 3;
const FOOTER: u16 = 3;
const BLOCK_HEIGHT: u16 = 14;
const LOT_WIDTH: u16 = 26;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CityTarget {
    pub(super) endpoint_id: ClientEndpointId,
    pub(super) boot_id: String,
    pub(super) pane_id: String,
}

#[derive(Debug, Default)]
pub(super) struct CityState {
    pub(super) selected: Option<CityTarget>,
    /// First visible city row, independent of terminal width.
    pub(super) scroll: usize,
}

pub(super) struct CityAgent {
    pub(super) target: CityTarget,
    pub(super) name: String,
    pub(super) detail: String,
    pub(super) status: AgentStatus,
    pub(super) stale: bool,
}

pub(super) struct CityLayout {
    pub(super) houses: Vec<(Rect, usize)>,
    pub(super) close: Rect,
    pub(super) columns: usize,
    pub(super) visible_rows: usize,
    pub(super) max_scroll: usize,
    pub(super) compact: bool,
}

fn body(area: Rect) -> Rect {
    let header = HEADER.min(area.height);
    Rect::new(
        area.x,
        area.y + header,
        area.width,
        area.height.saturating_sub(header + FOOTER),
    )
}

pub(super) fn layout(area: Rect, count: usize, scroll: usize) -> CityLayout {
    let body = body(area);
    let compact = body.height < BLOCK_HEIGHT || body.width < 24;
    let columns = if compact {
        1
    } else {
        (body.width / LOT_WIDTH).max(1) as usize
    };
    let row_height = if compact { 1 } else { BLOCK_HEIGHT };
    let visible_rows = (body.height / row_height) as usize;
    let max_scroll = count.div_ceil(columns).saturating_sub(visible_rows);
    let start = scroll.min(max_scroll).saturating_mul(columns);
    let end = start
        .saturating_add(visible_rows.saturating_mul(columns))
        .min(count);
    let mut houses = Vec::with_capacity(end.saturating_sub(start));
    for index in start..end {
        let slot = index - start;
        let col = (slot % columns) as u16;
        let row = (slot / columns) as u16;
        let width = body.width / columns as u16;
        let padding = if compact { 0 } else { 2 };
        houses.push((
            Rect::new(
                body.x + col * width + padding,
                body.y + row * row_height,
                width.saturating_sub(padding * 2),
                if compact { 1 } else { 10 },
            ),
            index,
        ));
    }
    let close_width = area.width.min(7);
    CityLayout {
        houses,
        close: Rect::new(
            area.right().saturating_sub(close_width),
            area.y,
            close_width,
            u16::from(area.height > 0),
        ),
        columns,
        visible_rows,
        max_scroll,
        compact,
    }
}

fn text(buffer: &mut Buffer, area: Rect, value: &str, style: Style) {
    let area = area.intersection(buffer.area);
    if !area.is_empty() {
        buffer.set_stringn(area.x, area.y, value, area.width as usize, style);
    }
}

fn line(area: Rect, row: u16) -> Rect {
    if row >= area.height {
        return Rect::default();
    }
    Rect::new(area.x, area.y + row, area.width, 1)
}

fn seed(target: &CityTarget) -> u64 {
    // A home's appearance stays fixed through status changes and server restarts.
    target
        .endpoint_id
        .storage_key()
        .bytes()
        .chain(target.pane_id.bytes())
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

pub(super) fn render(
    buffer: &mut Buffer,
    area: Rect,
    agents: &[CityAgent],
    state: &CityState,
    layout: &CityLayout,
    palette: &Palette,
) {
    let area = area.intersection(buffer.area);
    if area.is_empty() {
        return;
    }
    let base = Style::default().fg(palette.text).bg(palette.panel_bg);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            buffer[(x, y)].set_symbol(" ").set_style(base);
        }
    }
    let title_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(layout.close.width),
        1,
    );
    text(
        buffer,
        title_area,
        &format!(" AGENT CITY  ·  {} agents", agents.len()),
        base.add_modifier(Modifier::BOLD),
    );
    text(buffer, layout.close, " Esc × ", base.fg(palette.overlay0));
    text(
        buffer,
        line(area, 1),
        " Every house opens an agent's terminal.",
        base.fg(palette.overlay0),
    );
    let legend = [
        ("● working  ", AgentStatus::Working),
        ("● blocked  ", AgentStatus::Blocked),
        ("● done  ", AgentStatus::Done),
        ("○ idle", AgentStatus::Idle),
    ];
    let mut legend_x = area.x.saturating_add(1);
    for (label, status) in legend {
        let width = UnicodeWidthStr::width(label) as u16;
        if legend_x.saturating_add(width) > area.right() || area.height < 3 {
            break;
        }
        text(
            buffer,
            Rect::new(legend_x, area.y + 2, width, 1),
            label,
            base.fg(status_color(status, palette)),
        );
        legend_x += width;
    }
    let body = body(area);
    if !layout.compact {
        super::city_art::draw_ground(buffer, body);
        for row in 0..layout.visible_rows as u16 {
            super::city_art::draw_street(
                buffer,
                Rect::new(body.x, body.y + row * BLOCK_HEIGHT + 10, body.width, 4),
            );
        }
    }
    if agents.is_empty() {
        let style = if layout.compact {
            base
        } else {
            base.bg(Color::Rgb(57, 83, 48))
        };
        let y = body.y + body.height.saturating_sub(2) / 2;
        text(
            buffer,
            Rect::new(
                body.x + body.width.min(1),
                y,
                body.width.saturating_sub(2),
                u16::from(body.height > 0),
            ),
            " Your neighborhood is waiting.",
            style.add_modifier(Modifier::BOLD),
        );
        if y + 1 < body.bottom() {
            text(
                buffer,
                Rect::new(
                    body.x + body.width.min(1),
                    y + 1,
                    body.width.saturating_sub(2),
                    1,
                ),
                " Start an agent in any herdr pane.",
                style,
            );
        }
    }
    for (house, index) in &layout.houses {
        let Some(agent) = agents.get(*index) else {
            continue;
        };
        let selected = state.selected.as_ref() == Some(&agent.target);
        let accent = if agent.stale {
            palette.overlay0
        } else {
            status_color(agent.status, palette)
        };
        let label = if agent.stale {
            "offline"
        } else {
            status_text(agent.status)
        };
        let marker = if selected { "▸" } else { " " };
        if layout.compact {
            text(
                buffer,
                *house,
                &format!("{marker} {} · {label}", agent.name),
                base.fg(accent).add_modifier(if selected {
                    Modifier::BOLD | Modifier::REVERSED
                } else {
                    Modifier::empty()
                }),
            );
            continue;
        }
        super::city_art::draw_house(
            buffer,
            Rect::new(house.x, house.y, house.width, 8),
            seed(&agent.target),
            accent,
            selected,
        );
        let label_style = Style::default()
            .fg(if selected {
                palette.panel_bg
            } else {
                Color::Rgb(244, 241, 219)
            })
            .bg(if selected {
                accent
            } else {
                Color::Rgb(47, 65, 43)
            });
        text(
            buffer,
            line(*house, 8),
            &format!("{marker} {}", agent.name),
            label_style.add_modifier(Modifier::BOLD),
        );
        text(
            buffer,
            line(*house, 9),
            &format!(
                "  {} {label}",
                if agent.stale {
                    "×"
                } else {
                    status_dot(agent.status)
                }
            ),
            label_style.fg(if selected { palette.panel_bg } else { accent }),
        );
        if house.x >= body.x + 2 {
            super::city_art::draw_tree(
                buffer,
                Rect::new(house.x - 2, house.y + 5, 3, 3),
                *index as u64 + 4,
            );
        }
    }
    let footer_y = area.bottom().saturating_sub(FOOTER).max(body.bottom());
    if footer_y < area.bottom() {
        let selected = agents
            .iter()
            .find(|agent| state.selected.as_ref() == Some(&agent.target));
        if let Some(agent) = selected {
            text(
                buffer,
                Rect::new(area.x, footer_y, area.width, 1),
                &format!(" {}  ·  {}", agent.name, agent.detail),
                base.fg(palette.text),
            );
        }
        if footer_y + 1 < area.bottom() {
            text(
                buffer,
                Rect::new(area.x, footer_y + 1, area.width, 1),
                " ↑↓←→ move  ·  Enter / click open  ·  Esc back",
                base.fg(palette.overlay0),
            );
        }
        if footer_y + 2 < area.bottom() {
            let first = layout.houses.first().map_or(0, |(_, index)| index + 1);
            let last = layout.houses.last().map_or(0, |(_, index)| index + 1);
            let hint = if layout.max_scroll > 0 {
                "  ·  wheel / PgUp / PgDn to explore"
            } else {
                ""
            };
            text(
                buffer,
                Rect::new(area.x, footer_y + 2, area.width, 1),
                &format!(" {first}–{last} of {}{hint}", agents.len()),
                base.fg(palette.overlay0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_layout_stays_in_bounds_and_reaches_every_agent() {
        for width in [0, 1, 12, 23, 24, 52, 80, 160] {
            for height in [0, 1, 5, 10, 19, 20, 24, 50] {
                let area = Rect::new(3, 2, width, height);
                let first = layout(area, 37, 0);
                let mut visited = std::collections::HashSet::new();
                for scroll in 0..=first.max_scroll {
                    let current = layout(area, 37, scroll);
                    for (rect, index) in current.houses {
                        assert_eq!(rect.intersection(area), rect);
                        assert!(index < 37);
                        visited.insert(index);
                    }
                }
                if first.visible_rows > 0 {
                    assert_eq!(visited.len(), 37);
                }
            }
        }
    }

    #[test]
    fn city_layout_clamps_scroll_after_agent_removal() {
        let area = Rect::new(0, 0, 80, 24);
        let current = layout(area, 1, usize::MAX);
        assert_eq!(current.max_scroll, 0);
        assert_eq!(current.houses.len(), 1);
        assert_eq!(current.houses[0].1, 0);
    }

    fn sample_agents(count: usize) -> Vec<CityAgent> {
        let names = [
            "Frontend",
            "API builder",
            "Code review",
            "Tests",
            "Research",
            "Documentation",
            "Refactor",
            "Deploy",
        ];
        (0..count)
            .map(|index| CityAgent {
                target: CityTarget {
                    endpoint_id: ClientEndpointId::Local,
                    boot_id: "preview".into(),
                    pane_id: format!("pane_{}", index + 1),
                },
                name: names[index % names.len()].into(),
                detail: "local · new-era-agent".into(),
                status: [
                    AgentStatus::Working,
                    AgentStatus::Working,
                    AgentStatus::Blocked,
                    AgentStatus::Done,
                    AgentStatus::Idle,
                ][index % 5],
                stale: false,
            })
            .collect()
    }

    #[test]
    fn city_render_handles_tiny_buffers_and_long_unicode_names() {
        let mut agents = sample_agents(2);
        agents[0].name = "東京 🏡 a very long agent name".repeat(8);
        for width in [0, 1, 8, 24, 80] {
            for height in [0, 1, 3, 6, 12, 20, 40] {
                let area = Rect::new(0, 0, width, height);
                let mut buffer = Buffer::empty(area);
                let layout = layout(area, agents.len(), 0);
                render(
                    &mut buffer,
                    area,
                    &agents,
                    &CityState::default(),
                    &layout,
                    &Palette::catppuccin(),
                );
            }
        }
    }

    /// Manual visual inspection artifact, built from the actual terminal buffer.
    #[test]
    #[ignore]
    fn city_preview() {
        let agents = sample_agents(8);
        let area = Rect::new(0, 0, 112, 36);
        let mut buffer = Buffer::empty(area);
        let layout = layout(area, agents.len(), 0);
        let state = CityState {
            selected: Some(agents[2].target.clone()),
            ..Default::default()
        };
        render(
            &mut buffer,
            area,
            &agents,
            &state,
            &layout,
            &Palette::catppuccin(),
        );
        let frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
        let path = std::path::Path::new(".local/city-preview.json");
        std::fs::create_dir_all(".local").unwrap();
        std::fs::write(path, serde_json::to_vec(&frame).unwrap()).unwrap();
        eprintln!("City preview written to {}", path.display());
    }

    #[test]
    #[ignore]
    fn city_render_scale_profile() {
        let area = Rect::new(0, 0, 132, 48);
        for count in [1, 15] {
            let agents = sample_agents(count);
            let mut buffer = Buffer::empty(area);
            let layout = layout(area, agents.len(), 0);
            let state = CityState::default();
            let palette = Palette::catppuccin();
            let start = std::time::Instant::now();
            for _ in 0..200 {
                render(&mut buffer, area, &agents, &state, &layout, &palette);
                std::hint::black_box(&buffer);
            }
            eprintln!(
                "city {count} agents, 132×48: {:.1} µs/frame",
                start.elapsed().as_micros() as f64 / 200.0
            );
        }
    }
}
