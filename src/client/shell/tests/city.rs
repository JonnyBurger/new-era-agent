use super::*;

fn city_agent(pane_id: &str, name: &str) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: "tab_1".into(),
        name: Some(name.into()),
        display_agent: None,
        agent: Some("codex".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: pane_id == "pane_1",
    }
}

fn city_state(count: usize) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut projection = snapshot();
    projection.agents = (0..count)
        .map(|index| {
            city_agent(
                &format!("pane_{}", index + 1),
                &format!("Agent {}", index + 1),
            )
        })
        .collect();
    state.set_snapshot(Box::new(projection));
    state.set_pane_surface(surface());
    state
}

fn city_key(state: &mut ClientShellState, code: KeyCode) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        code,
        KeyModifiers::empty(),
    ))])
}

fn open_city(state: &mut ClientShellState) {
    state.mode = ClientShellMode::Prefix;
    let input = city_key(state, KeyCode::Char('a'));
    assert!(input.actions.is_empty());
    assert!(state.city.is_some());
    state.compose(120, 40).unwrap();
}

fn click_house(
    state: &mut ClientShellState,
    target: &crate::client::shell::city::CityTarget,
) -> ClientShellInput {
    let rect = state
        .hits
        .city_houses
        .iter()
        .find(|(_, candidate)| candidate == target)
        .unwrap()
        .0;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + rect.width / 2,
        row: rect.y + rect.height / 2,
        modifiers: KeyModifiers::empty(),
    })])
}

#[test]
fn city_shortcut_preserves_goto_and_escape_restores_terminal_input() {
    let mut state = city_state(2);
    state.mode = ClientShellMode::Prefix;
    city_key(&mut state, KeyCode::Char('g'));
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::Navigator(_))
    ));
    assert!(state.city.is_none());
    state.overlay = None;
    open_city(&mut state);
    let frame = state.compose(120, 40).unwrap();
    assert!(frame.cursor.is_none());
    assert!(state.hits.panes.is_empty());
    assert!(!frame_rows(&frame).iter().any(|row| row.contains("LIVE")));
    city_key(&mut state, KeyCode::Esc);
    assert!(state.city.is_none());
    let input = city_key(&mut state, KeyCode::Char('x'));
    assert!(input.requests.iter().any(|request| matches!(request, ClientMessage::ClientShellPaneInput { pane_id, .. } if pane_id == "pane_1")));
}

#[test]
fn city_uses_endpoint_qualified_house_targets() {
    let mut state = city_state(1);
    let profile = crate::client::endpoint::SavedSshEndpoint {
        id: crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef").unwrap(),
        label: "Build".into(),
        target: "dev@build.example".into(),
        session: "agents".into(),
        enabled: true,
    };
    let remote = ClientEndpointId::Ssh(profile.id.clone());
    state.set_endpoint_catalog(&[profile]);
    state.set_endpoint_status(&remote, ClientEndpointStatus::Online);
    let mut projection = snapshot();
    projection.boot_id = "remote-boot".into();
    projection.agents = vec![city_agent("pane_1", "Remote agent")];
    state.set_endpoint_snapshot(&remote, Box::new(projection));
    open_city(&mut state);
    assert_eq!(state.hits.city_houses.len(), 2);
    let target = state
        .hits
        .city_houses
        .iter()
        .find(|(_, target)| target.endpoint_id == remote)
        .unwrap()
        .1
        .clone();
    let input = click_house(&mut state, &target);
    assert!(
        matches!(input.actions.as_slice(), [ClientShellAction::ActivateEndpoint { endpoint_id, target: Some(ClientEndpointFocusTarget::Pane(pane_id)) }] if endpoint_id == &remote && pane_id == "pane_1")
    );
    assert!(state.city.is_none());
}

#[test]
fn city_rejects_selection_from_a_previous_server_boot() {
    let mut state = city_state(1);
    open_city(&mut state);
    let mut projection = snapshot();
    projection.boot_id = "new-boot".into();
    projection.agents = vec![city_agent("pane_1", "Replacement")];
    state.set_snapshot(Box::new(projection));
    let input = city_key(&mut state, KeyCode::Enter);
    assert!(input.actions.is_empty());
    assert!(state.city.is_some());
    assert!(state.visible_endpoint_notice.is_some());
}

#[test]
fn city_keyboard_reveals_and_focuses_offscreen_house() {
    let mut state = city_state(40);
    open_city(&mut state);
    city_key(&mut state, KeyCode::End);
    state.compose(120, 40).unwrap();
    assert!(state.city.as_ref().unwrap().scroll > 0);
    assert!(state
        .hits
        .city_houses
        .iter()
        .any(|(_, target)| target.pane_id == "pane_40"));
    let input = city_key(&mut state, KeyCode::Enter);
    assert!(input.actions.iter().any(|action| matches!(action, ClientShellAction::Endpoint { request, .. } if matches!(&request.method, crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_40"))));
    assert!(state.city.is_none());
}

#[test]
fn city_does_not_forward_paste_text_or_keys_to_hidden_popups() {
    let mut state = city_state(1);
    open_city(&mut state);
    state.popup_terminal_id = Some("popup-1".into());
    state.popup_pending = true;
    let input = state.handle_raw_events(vec![
        RawInputEvent::Text(crate::input::TextCommit::new("hidden")),
        RawInputEvent::Paste("hidden".into()),
        RawInputEvent::Key(crate::input::TerminalKey::new(
            KeyCode::Char('x'),
            KeyModifiers::empty(),
        )),
    ]);
    assert!(input.requests.is_empty());
    assert!(state.clipboard_image_target().is_none());
    city_key(&mut state, KeyCode::Esc);
    assert!(state.city.is_none());
}

#[test]
fn city_renders_while_waiting_for_a_resized_terminal_surface() {
    let mut state = city_state(2);
    open_city(&mut state);
    state.invalidate_pane_surface();
    let frame = state.compose(90, 24).unwrap();
    assert!(state.pane_surface.is_none());
    assert_eq!(state.hits.city_houses.len(), 2);
    assert!(frame_rows(&frame)
        .iter()
        .any(|row| row.to_lowercase().contains("city")));
}

#[test]
fn city_keeps_hidden_agent_completion_unread() {
    let mut state = city_state(1);
    open_city(&mut state);
    let mut projection = snapshot();
    projection.revision = 2;
    let mut agent = city_agent("pane_1", "Agent 1");
    agent.agent_status = AgentStatus::Idle;
    agent.state_change_seq = 2;
    projection.agents = vec![agent];
    state.set_snapshot(Box::new(projection));
    let mut completed = surface();
    completed.projection_revision = 2;
    completed.surface_revision = 2;
    state.set_pane_surface(completed.clone());
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Done
    );
    assert!(!state.acknowledge_active_surface_agents(&completed));
    city_key(&mut state, KeyCode::Esc);
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Idle
    );
}

#[test]
fn city_prefix_commands_remain_available() {
    let mut state = city_state(2);
    open_city(&mut state);
    state.handle_raw_events(vec![RawInputEvent::Key(crate::input::TerminalKey::new(
        KeyCode::Char('b'),
        KeyModifiers::CONTROL,
    ))]);
    assert_eq!(state.mode, ClientShellMode::Prefix);
    let input = city_key(&mut state, KeyCode::Char('q'));
    assert!(input.detach);
    assert!(state.city.is_some());
}

#[test]
fn city_focus_reveals_after_open_and_resize_without_fighting_wheel_scroll() {
    let mut state = city_state(40);
    let mut projection = state.snapshot.as_deref().unwrap().clone();
    projection.focused_pane_id = Some("pane_40".into());
    state.set_snapshot(Box::new(projection));
    open_city(&mut state);
    assert!(state
        .hits
        .city_houses
        .iter()
        .any(|(_, target)| target.pane_id == "pane_40"));
    state.compose(60, 24).unwrap();
    assert!(state
        .hits
        .city_houses
        .iter()
        .any(|(_, target)| target.pane_id == "pane_40"));
    let before = state.city.as_ref().unwrap().scroll;
    let area = state.hits.city_area;
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: area.x + 2,
        row: area.y + 4,
        modifiers: KeyModifiers::empty(),
    })]);
    state.compose(60, 24).unwrap();
    assert_eq!(state.city.as_ref().unwrap().scroll, before - 1);
}

#[test]
fn city_duplicate_snapshot_does_not_mark_a_hidden_completion_read() {
    let mut state = city_state(1);
    open_city(&mut state);
    let mut projection = snapshot();
    projection.revision = 2;
    let mut agent = city_agent("pane_1", "Agent 1");
    agent.agent_status = AgentStatus::Idle;
    agent.state_change_seq = 2;
    projection.agents = vec![agent];
    state.set_snapshot(Box::new(projection.clone()));
    let mut completed = surface();
    completed.projection_revision = 2;
    state.set_pane_surface(completed);
    state.set_snapshot(Box::new(projection));
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Done
    );
}

#[test]
fn city_renders_rejection_and_allows_dismissing_it() {
    let mut state = city_state(1);
    open_city(&mut state);
    state.receive_endpoint_unavailable("House is offline".into());
    let frame = state.compose(120, 40).unwrap();
    assert!(frame_rows(&frame)
        .iter()
        .any(|row| row.contains("House is offline")));
    let toast = state.hits.notification_toast;
    assert!(!toast.is_empty());
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: toast.x,
        row: toast.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.visible_endpoint_notice.is_none());
    assert!(state.city.is_some());
}

#[test]
fn city_mobile_switch_button_returns_to_existing_navigation() {
    let mut state = city_state(2);
    open_city(&mut state);
    state.compose(40, 24).unwrap();
    let button = state.hits.mobile_switch;
    assert!(!button.is_empty());
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: button.x,
        row: button.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(state.city.is_none());
    assert_eq!(state.mode, ClientShellMode::Navigate);
    state.compose(40, 24).unwrap();
    assert!(!state.hits.mobile_targets.is_empty());
}

#[test]
#[ignore = "writes a local full-shell screenshot fixture for visual inspection"]
fn city_shell_preview() {
    let mut state = city_state(8);
    open_city(&mut state);
    let frame = state.compose(140, 42).unwrap();
    std::fs::create_dir_all(".local").unwrap();
    std::fs::write(
        ".local/city-shell-preview.json",
        serde_json::to_vec(&frame.frame).unwrap(),
    )
    .unwrap();
}

#[test]
fn city_navigation_does_not_acknowledge_the_terminal_it_leaves() {
    let mut state = city_state(2);
    open_city(&mut state);
    let mut projection = state.snapshot.as_deref().unwrap().clone();
    projection.revision = 2;
    projection.agents[0].agent_status = AgentStatus::Idle;
    projection.agents[0].state_change_seq = 2;
    state.set_snapshot(Box::new(projection));
    let mut completed = surface();
    completed.projection_revision = 2;
    completed.surface_revision = 2;
    state.set_pane_surface(completed);
    state.compose(120, 40).unwrap();
    city_key(&mut state, KeyCode::Right);
    city_key(&mut state, KeyCode::Enter);
    assert!(state.city.is_none());
    assert_eq!(
        state.snapshot.as_ref().unwrap().agents[0].agent_status,
        AgentStatus::Done
    );
}
