//! Client-owned navigation for the agent city. No session or wire state lives here.
use super::*;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

impl ClientShellState {
    fn city_agents(&self) -> Vec<city::CityAgent> {
        // Preserve snapshot order: houses must not move when an agent changes status.
        self.endpoints
            .iter()
            .flat_map(|endpoint| {
                endpoint
                    .snapshot
                    .as_deref()
                    .into_iter()
                    .flat_map(move |snapshot| {
                        snapshot.agents.iter().map(move |agent| {
                            let name = agent
                                .name
                                .as_deref()
                                .or(agent.title.as_deref())
                                .or(agent.display_agent.as_deref())
                                .or(agent.agent.as_deref())
                                .unwrap_or(&agent.pane_id)
                                .to_owned();
                            let workspace = snapshot
                                .workspaces
                                .iter()
                                .find(|workspace| workspace.workspace_id == agent.workspace_id)
                                .map(|workspace| workspace.label.as_str())
                                .unwrap_or(&agent.workspace_id);
                            city::CityAgent {
                                target: city::CityTarget {
                                    endpoint_id: endpoint.endpoint_id.clone(),
                                    boot_id: snapshot.boot_id.clone(),
                                    pane_id: agent.pane_id.clone(),
                                },
                                name,
                                detail: format!("{} · {}", endpoint.label, workspace),
                                status: agent.agent_status,
                                stale: endpoint.status != ClientEndpointStatus::Online,
                            }
                        })
                    })
            })
            .collect()
    }

    pub(super) fn open_city(&mut self, outcome: &mut ClientShellInput) {
        self.release_input_leases(outcome);
        self.reset_copy_pipeline();
        self.stop_selection_autoscroll();
        self.selection = None;
        self.selection_highlight_clear_deadline = None;
        self.word_selection_gesture = None;
        self.last_pane_click = None;
        self.chrome_drag = None;
        self.workspace_press = None;
        self.tab_press = None;
        self.clear_link_hover();
        self.overlay = None;
        self.mode = ClientShellMode::Terminal;
        self.navigate_workspace_id = None;
        self.pending_workspace_highlight = None;
        self.city.get_or_insert_with(Default::default);
        self.city_reveal_selection = true;
        // Clear immediately: input can arrive before the next composition.
        self.hits.panes.clear();
        self.hits.pane_splits.clear();
        self.hits.popup = None;
        outcome.repaint = true;
    }

    pub(super) fn close_city(&mut self, outcome: &mut ClientShellInput) {
        if self.city.take().is_some() {
            self.hits.city_houses.clear();
            self.hits.city_area = Rect::default();
            self.hits.city_close = Rect::default();
            self.mode = self.copy_or_terminal_mode();
            outcome.repaint = true;
        }
    }

    fn return_from_city(&mut self, outcome: &mut ClientShellInput) {
        self.close_city(outcome);
        // Esc and the close controls reveal the retained terminal immediately. A
        // focus action instead waits for its destination surface to be presented.
        if self.pending_pane_surface.is_some()
            || self.pane_surface_generation != self.active_snapshot_generation
            || !self.endpoint_is_online(&self.active_endpoint_id)
        {
            return;
        }
        if let Some(surface) = self.pane_surface.take() {
            outcome.repaint |= self.acknowledge_active_surface_agents(&surface);
            self.pane_surface = Some(surface);
        }
    }

    fn city_scene(&mut self, area: Rect) -> (Vec<city::CityAgent>, city::CityLayout) {
        let agents = self.city_agents();
        let focused = self.focused_pane_id();
        let mut reveal = std::mem::take(&mut self.city_reveal_selection);
        let state = self.city.get_or_insert_with(Default::default);
        if !state
            .selected
            .as_ref()
            .is_some_and(|selected| agents.iter().any(|agent| &agent.target == selected))
        {
            reveal = true;
            state.selected = agents
                .iter()
                .find(|agent| {
                    agent.target.endpoint_id == self.active_endpoint_id
                        && focused.as_deref() == Some(agent.target.pane_id.as_str())
                })
                .or_else(|| agents.iter().find(|agent| !agent.stale))
                .or_else(|| agents.first())
                .map(|agent| agent.target.clone());
        }
        let layout = city::layout(area, agents.len(), state.scroll);
        if reveal {
            if let Some(index) = state
                .selected
                .as_ref()
                .and_then(|selected| agents.iter().position(|agent| &agent.target == selected))
            {
                let row = index / layout.columns.max(1);
                if row < state.scroll {
                    state.scroll = row;
                } else if row >= state.scroll + layout.visible_rows.max(1) {
                    state.scroll = row + 1 - layout.visible_rows.max(1);
                }
            }
        }
        state.scroll = state.scroll.min(layout.max_scroll);
        let layout = city::layout(area, agents.len(), state.scroll);
        (agents, layout)
    }

    fn city_area(&self) -> Rect {
        self.last_composed_size
            .map(|(cols, rows)| self.layout(cols, rows).pane_surface)
            .unwrap_or_default()
    }

    fn move_city_selection(&mut self, code: KeyCode) {
        let area = self.city_area();
        let (agents, layout) = self.city_scene(area);
        let Some(state) = self.city.as_mut() else {
            return;
        };
        if agents.is_empty() {
            return;
        }
        let index = state
            .selected
            .as_ref()
            .and_then(|selected| agents.iter().position(|agent| &agent.target == selected))
            .unwrap_or(0);
        let columns = layout.columns.max(1);
        let page = columns.saturating_mul(layout.visible_rows.max(1));
        let next = match code {
            KeyCode::Left | KeyCode::BackTab => index.saturating_sub(1),
            KeyCode::Right | KeyCode::Tab => (index + 1).min(agents.len() - 1),
            KeyCode::Up => index.saturating_sub(columns),
            KeyCode::Down => (index + columns).min(agents.len() - 1),
            KeyCode::PageUp => index.saturating_sub(page),
            KeyCode::PageDown => index.saturating_add(page).min(agents.len() - 1),
            KeyCode::Home => 0,
            KeyCode::End => agents.len() - 1,
            _ => index,
        };
        state.selected = Some(agents[next].target.clone());
        let row = next / columns;
        if row < state.scroll {
            state.scroll = row;
        } else if row >= state.scroll + layout.visible_rows.max(1) {
            state.scroll = row + 1 - layout.visible_rows.max(1);
        }
        state.scroll = state.scroll.min(layout.max_scroll);
    }

    fn activate_city_target(&mut self, target: &city::CityTarget, outcome: &mut ClientShellInput) {
        let current = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == target.endpoint_id)
            .is_some_and(|endpoint| {
                endpoint.status == ClientEndpointStatus::Online
                    && endpoint.snapshot.as_deref().is_some_and(|snapshot| {
                        snapshot.boot_id == target.boot_id
                            && snapshot
                                .agents
                                .iter()
                                .any(|agent| agent.pane_id == target.pane_id)
                    })
            });
        if !current {
            self.receive_endpoint_unavailable(
                "This agent is no longer available. Select a connected house.".into(),
            );
            outcome.repaint = true;
            return;
        }
        if self.focus_or_activate(
            target.endpoint_id.clone(),
            ClientEndpointFocusTarget::Pane(target.pane_id.clone()),
            outcome,
        ) {
            self.close_city(outcome);
        }
    }

    pub(super) fn route_city_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if let Some(binding) =
            crate::input::resolve_direct_binding(&self.config.keybinds.keybinds, key)
        {
            self.record_binding(binding, outcome);
            return;
        }
        if self.config.keybinds.matches_prefix(key) {
            self.mode = ClientShellMode::Prefix;
            outcome.repaint = true;
            return;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') && key.modifiers.is_empty() {
            self.return_from_city(outcome);
        } else if key.code == KeyCode::Enter && key.modifiers.is_empty() {
            let target = self.city.as_ref().and_then(|state| state.selected.clone());
            if let Some(target) = target {
                self.activate_city_target(&target, outcome);
            }
        } else if key.modifiers.is_empty()
            || key.modifiers == KeyModifiers::SHIFT && key.code == KeyCode::BackTab
        {
            self.move_city_selection(key.code);
            outcome.repaint = true;
        }
    }

    pub(super) fn handle_city_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        if self.overlay.is_some() {
            return false;
        }
        let point = (mouse.column, mouse.row);
        if self.city.is_some()
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && contains(self.hits.mobile_switch, point)
        {
            self.close_city(outcome);
            return false;
        }
        if contains(self.hits.notification_toast, point) {
            return false;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && contains(self.hits.city_launcher, point)
        {
            if self.city.is_some() {
                self.return_from_city(outcome);
            } else {
                self.open_city(outcome);
            }
            return true;
        }
        if self.city.is_none() || !contains(self.hits.city_area, point) {
            return false;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if contains(self.hits.city_close, point) {
                    self.return_from_city(outcome);
                } else if let Some(target) = self
                    .hits
                    .city_houses
                    .iter()
                    .find(|(rect, _)| contains(*rect, point))
                    .map(|(_, target)| target.clone())
                {
                    self.activate_city_target(&target, outcome);
                }
            }
            MouseEventKind::Moved => {
                if let Some(target) = self
                    .hits
                    .city_houses
                    .iter()
                    .find(|(rect, _)| contains(*rect, point))
                    .map(|(_, target)| target.clone())
                {
                    if let Some(state) = self.city.as_mut() {
                        if state.selected.as_ref() != Some(&target) {
                            state.selected = Some(target);
                            outcome.repaint = true;
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let area = self.city_area();
                let (_, layout) = self.city_scene(area);
                if let Some(state) = self.city.as_mut() {
                    state.scroll = if mouse.kind == MouseEventKind::ScrollUp {
                        state.scroll.saturating_sub(1)
                    } else {
                        state.scroll.saturating_add(1).min(layout.max_scroll)
                    };
                }
                outcome.repaint = true;
            }
            _ => {}
        }
        true
    }

    pub(super) fn compose_city(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> Option<crate::client::frame_output::ComposedFrame> {
        let layout = self.layout(cols, rows);
        let (agents, city_layout) = self.city_scene(layout.pane_surface);
        let mut buffer = Buffer::empty(Rect::new(0, 0, cols, rows));
        self.hits = if let Some(snapshot) = self.snapshot.as_deref() {
            render::render_shell(
                &mut buffer,
                layout,
                snapshot,
                &self.config,
                render::ShellRenderState {
                    machine_diagnostics: &self.machine_diagnostics,
                    endpoints: &self.endpoints,
                    active_endpoint_id: &self.active_endpoint_id,
                    collapsed_endpoints: &self.collapsed_endpoints,
                    collapsed_groups: &self.collapsed_groups,
                    remote_collapsed_groups: &self.remote_collapsed_groups,
                    workspace_scroll: &mut self.workspace_scroll,
                    agent_scroll: &mut self.agent_scroll,
                    tab_scroll: &mut self.tab_scroll,
                    reveal_focused_workspace: &mut self.reveal_focused_workspace,
                    reveal_focused_tab: &mut self.reveal_focused_tab,
                    sidebar_collapsed: self.sidebar_collapsed,
                    sidebar_section_split: self.sidebar_section_split,
                    tab_drag_insert_index: None,
                    selected_workspace_id: None,
                    reveal_navigation_workspace: &mut self.reveal_navigation_workspace,
                    dragged_workspace_id: None,
                    workspace_drop_indicator_row: None,
                },
            )
        } else {
            ShellHitMap::default()
        };
        city::render(
            &mut buffer,
            layout.pane_surface,
            &agents,
            self.city.as_ref()?,
            &city_layout,
            &self.config.palette,
        );
        self.hits.city_area = layout.pane_surface;
        self.hits.city_close = city_layout.close;
        self.hits.city_houses = city_layout
            .houses
            .iter()
            .filter_map(|(rect, index)| {
                agents
                    .get(*index)
                    .map(|agent| (*rect, agent.target.clone()))
            })
            .collect();
        if let Some(error) = self.endpoint_error.as_deref() {
            render::put_text(
                &mut buffer,
                layout.pane_surface.x,
                layout.pane_surface.bottom().saturating_sub(1),
                layout.pane_surface.width,
                error,
                Style::default().fg(self.config.palette.red),
            );
        }
        let mut frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
        let mut occlusion = crate::kitty_graphics::surface::Occlusion::default();
        occlusion.cover(layout.pane_surface);
        if self.overlay.is_none() && self.mode != ClientShellMode::Terminal {
            render::render_mode_bar(
                &mut buffer,
                layout.pane_surface,
                self.mode,
                self.copy_mode.as_ref(),
                None,
                false,
                &self.config.keybinds,
                &self.config.palette,
            );
            frame = FrameData::from_ratatui_buffer_with_hyperlinks(&buffer, None, &[]);
        }
        self.compose_notifications(&mut frame, layout, cols, rows, &mut occlusion)?;
        self.compose_client_overlay(&mut frame, &mut occlusion)?;
        let graphics = self.compose_graphics(layout, &occlusion);
        Some(crate::client::frame_output::ComposedFrame { frame, graphics })
    }
}

pub(super) fn render_city_launcher(
    buffer: &mut Buffer,
    area: Rect,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    if area.width < 3 || area.height == 0 {
        return;
    }
    let available = hits.sidebar_toggle.x.saturating_sub(area.x);
    if available == 0 {
        return;
    }
    let rect = Rect::new(area.x, area.bottom().saturating_sub(1), available.min(8), 1);
    render::put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width,
        if rect.width >= 6 { "⌂ city" } else { "⌂" },
        Style::default()
            .fg(config.palette.accent)
            .bg(config.palette.sidebar_bg),
    );
    if config.mouse_capture {
        hits.city_launcher = rect;
    }
}
