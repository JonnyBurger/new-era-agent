use super::*;

pub(crate) struct ClientComposedSurfacePatch {
    pub(crate) rows: Vec<crate::protocol::PaneSurfacePatchRow>,
    pub(crate) cursor: Option<crate::protocol::CursorState>,
}

pub(crate) enum ClientPaneSurfacePatchOutcome {
    Rejected,
    Applied(Option<ClientComposedSurfacePatch>),
    /// The retained terminal is current, but no visible cells need presenting.
    AppliedHidden,
}

fn row_fits_frame(row: &crate::protocol::PaneSurfacePatchRow, frame: &FrameData) -> bool {
    row.x
        .saturating_add(row.cells.len().min(u16::MAX as usize) as u16)
        <= frame.width
        && row.y < frame.height
}

fn apply_row(row: &crate::protocol::PaneSurfacePatchRow, frame: &mut FrameData) -> bool {
    if !row_fits_frame(row, frame) {
        return false;
    }
    let start = usize::from(row.y) * usize::from(frame.width) + usize::from(row.x);
    let end = start + row.cells.len();
    if end > frame.cells.len() {
        return false;
    }
    frame.cells[start..end].clone_from_slice(&row.cells);
    true
}

fn apply_patch_to_surface(
    surface: &mut crate::protocol::PaneSurfaceFrame,
    patch: &crate::protocol::PaneSurfacePatch,
) -> bool {
    for row in &patch.rows {
        if !apply_row(row, &mut surface.frame) {
            return false;
        }
    }
    for updated in &patch.panes {
        let Some(existing) = surface
            .panes
            .iter_mut()
            .find(|pane| pane.pane_id == updated.pane_id)
        else {
            return false;
        };
        *existing = updated.clone();
    }
    surface.frame.cursor = patch.cursor.clone();
    surface.surface_revision = patch.surface_revision;
    true
}

fn fast_path_blocker(
    state: &ClientShellState,
    patch: &crate::protocol::PaneSurfacePatch,
) -> Option<&'static str> {
    if state.mode != ClientShellMode::Terminal {
        Some("client_surface_patch.fallback.mode")
    } else if state.overlay.is_some() {
        Some("client_surface_patch.fallback.overlay")
    } else if state.endpoint_error.is_some() {
        Some("client_surface_patch.fallback.endpoint_error")
    } else if state.config_diagnostic.is_some() {
        Some("client_surface_patch.fallback.config_diagnostic")
    } else if state.visible_endpoint_notice.is_some() {
        Some("client_surface_patch.fallback.endpoint_notice")
    } else if state.visible_notification.is_some() {
        Some("client_surface_patch.fallback.notification")
    } else if state.copy_feedback.is_some() {
        Some("client_surface_patch.fallback.copy_feedback")
    } else if state.link_hover_blocks_patch(patch) {
        Some("client_surface_patch.fallback.link_hover")
    } else if state.selection.is_some() {
        Some("client_surface_patch.fallback.selection")
    } else if state.copy_mode.is_some() {
        Some("client_surface_patch.fallback.copy_mode")
    } else if state.selection_highlight_clear_deadline.is_some() {
        Some("client_surface_patch.fallback.selection_deadline")
    } else if patch.panes.iter().any(|pane| {
        !state
            .hits
            .panes
            .iter()
            .any(|hit| hit.pane_id == pane.pane_id)
    }) {
        Some("client_surface_patch.fallback.pane_hits")
    } else {
        None
    }
}

fn pane_geometry_matches(
    left: &crate::protocol::PaneSurfacePane,
    right: &crate::protocol::PaneSurfacePane,
) -> bool {
    left.pane_id == right.pane_id
        && left.rect == right.rect
        && left.inner_rect == right.inner_rect
        && left.focused == right.focused
        && left.pixel_width == right.pixel_width
        && left.pixel_height == right.pixel_height
}

impl ClientShellState {
    pub(crate) fn receive_pane_surface(
        &mut self,
        surface: PaneSurfaceFrame,
        cols: u16,
        rows: u16,
    ) -> Option<crate::client::frame_output::ComposedFrame> {
        // Full surfaces still update retained terminal and graphics state. A
        // speculative future surface may clear terminal hit targets; preserve
        // City's current hit map because that surface is not being presented.
        let city_hits = self.city.is_some().then(|| std::mem::take(&mut self.hits));
        let overlay_before = self.overlay.as_ref().map(std::mem::discriminant);
        let error_before = self.endpoint_error.is_some();
        let mode_before = self.mode;
        self.set_pane_surface(surface);
        if let Some(hits) = city_hits {
            self.hits = hits;
            // Popup transitions can dismiss a client overlay, notice or mode
            // bar. Those changes still need painting while terminals hide.
            if overlay_before == self.overlay.as_ref().map(std::mem::discriminant)
                && error_before == self.endpoint_error.is_some()
                && mode_before == self.mode
            {
                crate::render_prof::event("client_surface.hidden.city");
                return None;
            }
        }
        self.compose(cols, rows)
    }

    pub(crate) fn apply_pane_surface_patch(
        &mut self,
        patch: crate::protocol::PaneSurfacePatch,
    ) -> ClientPaneSurfacePatchOutcome {
        let Some(current) = self.pane_surface.as_ref() else {
            return ClientPaneSurfacePatchOutcome::Rejected;
        };
        if self.pane_surface_generation != self.active_snapshot_generation
            || patch.boot_id != current.boot_id
            || patch.projection_revision != current.projection_revision
            || patch.base_surface_revision != current.surface_revision
            || patch.surface_revision != current.surface_revision.saturating_add(1)
            || current.popup.is_some()
            || !current.graphics.placements.is_empty()
            || !current.graphics.retained_assets.is_empty()
        {
            return ClientPaneSurfacePatchOutcome::Rejected;
        }

        for updated in &patch.panes {
            let Some(existing) = current
                .panes
                .iter()
                .find(|pane| pane.pane_id == updated.pane_id)
            else {
                return ClientPaneSurfacePatchOutcome::Rejected;
            };
            if !pane_geometry_matches(existing, updated) {
                return ClientPaneSurfacePatchOutcome::Rejected;
            }
        }
        for row in &patch.rows {
            if !row_fits_frame(row, &current.frame)
                || row.cells.is_empty()
                || !patch.panes.iter().any(|pane| {
                    let terminal_row = row.x >= pane.inner_rect.x
                        && row.y >= pane.inner_rect.y
                        && row.y < pane.inner_rect.y.saturating_add(pane.inner_rect.height)
                        && row
                            .x
                            .saturating_add(row.cells.len().min(u16::MAX as usize) as u16)
                            <= pane.inner_rect.x.saturating_add(pane.inner_rect.width);
                    let scrollbar_rect = pane.scrollbar_rect.or_else(|| {
                        current
                            .panes
                            .iter()
                            .find(|existing| existing.pane_id == pane.pane_id)
                            .and_then(|existing| existing.scrollbar_rect)
                    });
                    let scrollbar_row = scrollbar_rect.is_some_and(|rect| {
                        row.x == rect.x
                            && row.y >= rect.y
                            && row.y < rect.y.saturating_add(rect.height)
                            && row.cells.len() == usize::from(rect.width)
                    });
                    terminal_row || scrollbar_row
                })
            {
                return ClientPaneSurfacePatchOutcome::Rejected;
            }
        }

        if self.city.is_some() {
            // Agent metadata arrives in snapshots. Terminal output is hidden by
            // City, so retain it without composing the city or allocating a
            // translated output patch. Copy mode still needs its existing
            // content/search reconciliation while it is suspended behind City.
            if self.copy_mode.is_some()
                || self.selection.is_some()
                || self.word_selection_gesture.is_some()
            {
                let mut next = current.clone();
                if !apply_patch_to_surface(&mut next, &patch) {
                    return ClientPaneSurfacePatchOutcome::Rejected;
                }
                let city_hits = std::mem::take(&mut self.hits);
                self.set_pane_surface(next);
                self.hits = city_hits;
            } else {
                let applied = self
                    .pane_surface
                    .as_mut()
                    .is_some_and(|surface| apply_patch_to_surface(surface, &patch));
                if !applied {
                    return ClientPaneSurfacePatchOutcome::Rejected;
                }
                for updated in &patch.panes {
                    self.acknowledge_patch_scroll_target(updated);
                }
                self.invalidate_link_hover();
                self.reconcile_input_source();
            }
            crate::render_prof::event("client_surface_patch.hidden.city");
            return ClientPaneSurfacePatchOutcome::AppliedHidden;
        }

        let fast_path_blocker = fast_path_blocker(self, &patch);
        if let Some(reason) = fast_path_blocker {
            crate::render_prof::event(reason);
        }
        let fast_path_area = fast_path_blocker.is_none().then(|| {
            let (cols, rows) = self.last_composed_size.unwrap_or_default();
            self.layout(cols, rows).pane_surface
        });
        let composed_patch = fast_path_area.map(|area| ClientComposedSurfacePatch {
            rows: patch
                .rows
                .iter()
                .map(|row| crate::protocol::PaneSurfacePatchRow {
                    x: area.x.saturating_add(row.x),
                    y: area.y.saturating_add(row.y),
                    cells: row.cells.clone(),
                })
                .collect(),
            cursor: patch
                .cursor
                .clone()
                .map(|cursor| crate::protocol::CursorState {
                    x: area.x.saturating_add(cursor.x),
                    y: area.y.saturating_add(cursor.y),
                    visible: cursor.visible,
                    shape: cursor.shape,
                }),
        });
        if let Some(area) = fast_path_area {
            let applied = self
                .pane_surface
                .as_mut()
                .is_some_and(|surface| apply_patch_to_surface(surface, &patch));
            if !applied {
                return ClientPaneSurfacePatchOutcome::Rejected;
            }
            for updated in &patch.panes {
                let Some(hit) = self
                    .hits
                    .panes
                    .iter_mut()
                    .find(|hit| hit.pane_id == updated.pane_id)
                else {
                    continue;
                };
                hit.scrollbar_rect = updated.scrollbar_rect.map(|rect| {
                    Rect::new(
                        area.x.saturating_add(rect.x),
                        area.y.saturating_add(rect.y),
                        rect.width,
                        rect.height,
                    )
                });
                hit.scroll = updated.scroll.map(|metrics| crate::pane::ScrollMetrics {
                    offset_from_bottom: usize::try_from(metrics.offset_from_bottom)
                        .unwrap_or(usize::MAX),
                    max_offset_from_bottom: usize::try_from(metrics.max_offset_from_bottom)
                        .unwrap_or(usize::MAX),
                    viewport_rows: usize::try_from(metrics.viewport_rows).unwrap_or(usize::MAX),
                });
                hit.mouse_reporting = updated.mouse_reporting;
                hit.sgr_pixel_mouse = updated.sgr_pixel_mouse;
                hit.pixel_width = updated.pixel_width;
                hit.pixel_height = updated.pixel_height;
                self.acknowledge_patch_scroll_target(updated);
            }
            self.invalidate_link_hover();
            self.reconcile_input_source();
        } else {
            let mut next = current.clone();
            if !apply_patch_to_surface(&mut next, &patch) {
                return ClientPaneSurfacePatchOutcome::Rejected;
            }
            self.set_pane_surface(next);
        }
        ClientPaneSurfacePatchOutcome::Applied(composed_patch)
    }

    fn acknowledge_patch_scroll_target(&mut self, updated: &crate::protocol::PaneSurfacePane) {
        if let (Some(target), Some(scroll)) = (
            self.pane_scroll_targets.get(&updated.pane_id).copied(),
            updated.scroll,
        ) {
            let target =
                target.min(usize::try_from(scroll.max_offset_from_bottom).unwrap_or(usize::MAX));
            if usize::try_from(scroll.offset_from_bottom).unwrap_or(usize::MAX) == target {
                self.pane_scroll_targets.remove(&updated.pane_id);
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn apply_composed_surface_patch(
    frame: &FrameData,
    patch: ClientComposedSurfacePatch,
) -> Option<FrameData> {
    let mut next = frame.clone();
    for row in &patch.rows {
        if !apply_row(row, &mut next) {
            return None;
        }
    }
    next.cursor = patch.cursor;
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::shell::tests::{snapshot, surface};
    use crate::protocol::{PaneSurfacePatch, PaneSurfacePatchRow, PaneSurfaceScrollMetrics};

    fn state_with_surface() -> ClientShellState {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
        state.set_snapshot(Box::new(snapshot()));
        let mut surface = surface();
        surface.panes[0].scroll = Some(PaneSurfaceScrollMetrics {
            offset_from_bottom: 0,
            max_offset_from_bottom: 8,
            viewport_rows: 2,
        });
        state.set_pane_surface(surface);
        state.compose(100, 30).expect("initial terminal frame");
        state
    }

    fn patch(state: &ClientShellState, symbol: &str) -> PaneSurfacePatch {
        let current = state.pane_surface.as_ref().expect("retained surface");
        let mut pane = current.panes[0].clone();
        pane.content_revision += 1;
        pane.mouse_reporting = true;
        let mut cell = current.frame.cells[0].clone();
        cell.symbol = symbol.into();
        PaneSurfacePatch {
            boot_id: current.boot_id.clone(),
            projection_revision: current.projection_revision,
            base_surface_revision: current.surface_revision,
            surface_revision: current.surface_revision + 1,
            rows: vec![PaneSurfacePatchRow {
                x: 0,
                y: 0,
                cells: vec![cell],
            }],
            panes: vec![pane],
            cursor: current.frame.cursor.clone(),
        }
    }

    fn popup_surface() -> PaneSurfaceFrame {
        let mut updated = surface();
        updated.surface_revision = 2;
        updated.frame.cells[0].symbol = "F".into();
        updated.popup = Some(Box::new(crate::protocol::ClientShellPopupSurface {
            terminal_id: "hidden-popup".into(),
            title: "Hidden popup".into(),
            width: None,
            height: None,
            frame: updated.frame.clone(),
            mouse_reporting: true,
            sgr_pixel_mouse: false,
            pixel_width: 0,
            pixel_height: 0,
        }));
        updated
    }

    #[test]
    fn city_retains_consecutive_hidden_patches_until_terminal_is_revealed() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        state.compose(100, 30).expect("city frame");
        let city_area = state.hits.city_area;
        let composed_at = state.last_composed_at;
        let cells = state.pane_surface.as_ref().unwrap().frame.cells.as_ptr();
        state.pane_scroll_targets.insert("pane_1".into(), 0);
        for symbol in ["A", "B"] {
            let patch = patch(&state, symbol);
            assert!(matches!(
                state.apply_pane_surface_patch(patch),
                ClientPaneSurfacePatchOutcome::AppliedHidden
            ));
            assert_eq!(
                state.pane_surface.as_ref().unwrap().frame.cells.as_ptr(),
                cells
            );
            assert_eq!(state.hits.city_area, city_area);
            assert!(state.hits.panes.is_empty());
            assert_eq!(state.last_composed_at, composed_at);
        }
        assert!(!state.pane_scroll_targets.contains_key("pane_1"));
        assert_eq!(state.pane_surface.as_ref().unwrap().surface_revision, 3);
        state.close_city(&mut ClientShellInput::default());
        let frame = state.compose(100, 30).expect("revealed terminal");
        let area = state.layout(100, 30).pane_surface;
        let index = usize::from(area.y) * usize::from(frame.width) + usize::from(area.x);
        assert_eq!(frame.cells[index].symbol, "B");
        assert!(state.hits.panes[0].mouse_reporting);
    }

    #[test]
    fn city_rejects_stale_hidden_patch_without_changing_retained_terminal() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        let before = state.pane_surface.clone();
        let mut patch = patch(&state, "X");
        patch.base_surface_revision = 0;
        assert!(matches!(
            state.apply_pane_surface_patch(patch),
            ClientPaneSurfacePatchOutcome::Rejected
        ));
        assert_eq!(state.pane_surface, before);
    }

    #[test]
    fn city_hidden_patch_reconciles_preserved_copy_search() {
        let mut state = state_with_surface();
        assert!(state.enter_copy_mode(&mut ClientShellInput::default()));
        let copy = state.copy_mode.as_mut().unwrap();
        copy.search_total = 3;
        copy.search_current = Some(0);
        copy.copy_after_search = true;
        state.open_city(&mut ClientShellInput::default());
        state.compose(100, 30).expect("city frame");
        let patch = patch(&state, "C");
        assert!(matches!(
            state.apply_pane_surface_patch(patch),
            ClientPaneSurfacePatchOutcome::AppliedHidden
        ));
        let copy = state.copy_mode.as_ref().expect("preserved copy mode");
        assert_eq!(copy.content_revision, 1);
        assert_eq!(copy.search_generation, 1);
        assert_eq!(copy.search_total, 0);
        assert_eq!(copy.search_current, None);
        assert!(!copy.copy_after_search);
        state.close_city(&mut ClientShellInput::default());
        assert_eq!(state.mode, ClientShellMode::Copy);
    }

    #[test]
    fn city_retains_full_terminal_and_popup_surfaces_without_presenting_them() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        state.compose(100, 30).expect("city frame");
        let composed_at = state.last_composed_at;
        let city_area = state.hits.city_area;
        assert!(state
            .receive_pane_surface(popup_surface(), 100, 30)
            .is_none());
        assert_eq!(state.last_composed_at, composed_at);
        assert_eq!(state.hits.city_area, city_area);
        assert!(state.hits.popup.is_none());
        assert_eq!(state.popup_terminal_id.as_deref(), Some("hidden-popup"));
        assert_eq!(
            state.pane_surface.as_ref().unwrap().frame.cells[0].symbol,
            "F"
        );
        state.close_city(&mut ClientShellInput::default());
        state.compose(100, 30).expect("revealed popup");
        assert!(state.hits.popup.is_some());
    }

    #[test]
    fn city_full_surface_repaints_when_popup_dismisses_a_visible_overlay() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        state.overlay = Some(ClientShellOverlay::Help(ClientHelpOverlay {
            query: TextEditor::default(),
            search_focused: true,
            scroll: 0,
        }));
        state.compose(100, 30).expect("city with help overlay");
        assert!(state
            .receive_pane_surface(popup_surface(), 100, 30)
            .is_some());
        assert!(!matches!(state.overlay, Some(ClientShellOverlay::Help(_))));
    }

    #[test]
    fn city_full_surface_repaints_when_popup_clears_prefix_mode() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        state.mode = ClientShellMode::Prefix;
        let before = state.compose(100, 30).expect("city with prefix mode bar");
        let after = state
            .receive_pane_surface(popup_surface(), 100, 30)
            .expect("repaint cleared prefix mode bar");
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(state.city.is_some());
        assert_ne!(before.cells, after.cells);
    }

    #[test]
    fn city_preserves_hit_targets_while_a_future_full_surface_waits() {
        let mut state = state_with_surface();
        state.open_city(&mut ClientShellInput::default());
        state.compose(100, 30).expect("city frame");
        let city_area = state.hits.city_area;
        let city_close = state.hits.city_close;
        let mut future = surface();
        future.projection_revision = 2;
        assert!(state.receive_pane_surface(future, 100, 30).is_none());
        assert!(state.pending_pane_surface.is_some());
        assert_eq!(state.hits.city_area, city_area);
        assert_eq!(state.hits.city_close, city_close);
    }
}
