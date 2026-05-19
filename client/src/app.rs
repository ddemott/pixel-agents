use std::collections::HashMap;

use anyhow::Result;
use futures_util::StreamExt;
use ratatui::crossterm::event::{
    Event, EventStream, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use serde_json::json;
use tokio::time::{interval, Duration, Instant};

use crate::agents::{parse_agent_list, AgentState, AgentStatus};
use crate::chrome::{self, ChromeAction};
use crate::daemon::wire::{ClientCapabilities, Inbound, Req};
use crate::daemon::{framing::Frame, DaemonConn};
use crate::focus::{tab_press, FocusMode, TabOutcome};
use crate::keymap::{Action, Keymap};
use crate::reconnect::{self, ReconnectState};
use crate::tui::Tui;

const FRAME_INTERVAL: Duration = Duration::from_millis(17); // ~60 fps

/// Discriminates in-flight RPC requests so responses can be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Subscribe,
    AgentList,
    AgentSpawn,
}

/// Status message shown in the main area (auto-clears after a few seconds).
#[derive(Debug)]
struct StatusMsg {
    text: String,
    until: Instant,
}

/// All mutable client state.
struct AppState {
    focus: FocusMode,
    agents: Vec<AgentState>,
    zoom: u8,
    keymap: Keymap,
    req_id: u32,
    pending: HashMap<u32, PendingKind>,
    hit_rects: Vec<chrome::HitRect>,
    status: Option<StatusMsg>,
    connected_boot_id: Option<String>,
    pub reconnect: Option<ReconnectState>,
}

impl AppState {
    fn new(keymap: Keymap) -> Self {
        Self {
            focus: FocusMode::Office,
            agents: Vec::new(),
            zoom: 2,
            keymap,
            req_id: 0,
            pending: HashMap::new(),
            hit_rects: Vec::new(),
            status: None,
            connected_boot_id: None,
            reconnect: None,
        }
    }

    fn next_req_id(&mut self) -> u32 {
        self.req_id += 1;
        self.req_id
    }

    fn zoom_in(&mut self) {
        if self.zoom < 10 {
            self.zoom += 1;
        }
    }

    fn zoom_out(&mut self) {
        if self.zoom > 1 {
            self.zoom -= 1;
        }
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMsg {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    fn agent_ids(&self) -> Vec<i32> {
        self.agents.iter().map(|a| a.id()).collect()
    }

    fn find_agent_mut(&mut self, id: i32) -> Option<&mut AgentState> {
        self.agents.iter_mut().find(|a| a.id() == id)
    }

    fn is_reconnecting(&self) -> bool {
        self.reconnect.is_some()
    }

    /// Called when the socket closes. Clears pending RPCs, starts retry timer.
    fn on_disconnected(&mut self) {
        self.pending.clear();
        self.reconnect = Some(ReconnectState::new());
    }

    /// Called on successful (re)connect. If bootId changed, clears agent state.
    fn on_connected(&mut self, boot_id: String) {
        let changed = self
            .connected_boot_id
            .as_deref()
            .map(|prev| prev != boot_id.as_str())
            .unwrap_or(false);

        if changed {
            self.agents.clear();
            if matches!(self.focus, FocusMode::PtyAgent(_)) {
                self.focus = FocusMode::Office;
            }
        }

        self.connected_boot_id = Some(boot_id);
        self.reconnect = None;
    }
}

/// Await the next frame, pending forever when not connected.
async fn recv_from_conn(conn: &mut Option<DaemonConn>) -> Result<Frame> {
    match conn.as_mut() {
        Some(c) => c.recv_frame().await,
        None => std::future::pending().await,
    }
}

pub async fn run(caps: ClientCapabilities) -> Result<()> {
    let keymap = Keymap::load();
    let mut state = AppState::new(keymap);
    let mut tui = Tui::new()?;
    let mut events = EventStream::new();
    let mut tick = interval(FRAME_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sigwinch = Sigwinch::new()?;

    // Trigger an immediate connect attempt on the first loop iteration.
    state.reconnect = Some(ReconnectState::new_immediate());
    let mut conn: Option<DaemonConn> = None;

    loop {
        // Expire status messages
        if let Some(ref s) = state.status {
            if Instant::now() >= s.until {
                state.status = None;
            }
        }

        // Fire reconnect attempt if the timer is due
        if let Some(ref rs) = state.reconnect {
            if rs.is_due() {
                match crate::daemon::connect(caps.clone()).await {
                    Ok(new_conn) => {
                        let boot_id = new_conn.boot_id.clone();
                        conn = Some(new_conn);
                        state.on_connected(boot_id);
                        let c = conn.as_mut().unwrap();
                        send_subscribe(c, &mut state).await?;
                        send_agent_list(c, &mut state).await?;
                    }
                    Err(_) => {
                        let rs = state.reconnect.as_mut().unwrap();
                        if rs.should_fork() {
                            rs.fork_attempted = true;
                            reconnect::try_fork_daemon();
                        }
                        rs.on_failure();
                    }
                }
                tui.draw(|f| render(f, &mut state))?;
            }
        }

        tokio::select! {
            // Daemon frames (only when connected)
            frame = recv_from_conn(&mut conn) => {
                match frame {
                    Ok(Frame::Ndjson(json)) => {
                        if let Err(e) = handle_daemon_json(&json, &mut state, conn.as_mut().unwrap()).await {
                            drop(tui);
                            return Err(e);
                        }
                    }
                    Ok(_) => {} // binary frames (PTY, asset) handled in Phase 3+
                    Err(_) => {
                        conn = None;
                        state.on_disconnected();
                    }
                }
            }

            // Wake up when the reconnect timer fires (guard disables this arm when connected)
            _ = tokio::time::sleep_until(
                state.reconnect.as_ref().map(|rs| rs.next_try).unwrap_or_else(|| Instant::now() + Duration::from_secs(3600))
            ), if state.is_reconnecting() => {
                // Actual attempt fires at top of next loop iteration via is_due()
            }

            // Crossterm input / resize / paste events
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if handle_event(event, &mut state, conn.as_mut()).await? == AppAction::Quit {
                            return Ok(());
                        }
                    }
                    Some(Err(e)) => {
                        drop(tui);
                        return Err(anyhow::anyhow!("terminal event error: {e}"));
                    }
                    None => return Ok(()),
                }
            }

            // SIGWINCH (no-op future on non-unix)
            _ = sigwinch.recv() => {
                tui.draw(|f| render(f, &mut state))?;
            }

            // Frame tick at ~60 fps
            _ = tick.tick() => {
                tui.draw(|f| render(f, &mut state))?;
            }
        }
    }
}

async fn send_subscribe(conn: &mut DaemonConn, state: &mut AppState) -> Result<()> {
    let id = state.next_req_id();
    state.pending.insert(id, PendingKind::Subscribe);
    let req = Req {
        kind: "req",
        req_id: id,
        method: "subscribe".into(),
        params: Some(json!({
            "topics": ["agent.created", "agent.exited", "agent.statusChanged"]
        })),
    };
    conn.send(&req).await
}

async fn send_agent_list(conn: &mut DaemonConn, state: &mut AppState) -> Result<()> {
    let id = state.next_req_id();
    state.pending.insert(id, PendingKind::AgentList);
    let req = Req {
        kind: "req",
        req_id: id,
        method: "agent.list".into(),
        params: None,
    };
    conn.send(&req).await
}

async fn send_agent_spawn(conn: &mut DaemonConn, state: &mut AppState) -> Result<()> {
    let id = state.next_req_id();
    state.pending.insert(id, PendingKind::AgentSpawn);
    let req = Req {
        kind: "req",
        req_id: id,
        method: "agent.spawn".into(),
        params: None,
    };
    conn.send(&req).await
}

async fn handle_daemon_json(
    json: &str,
    state: &mut AppState,
    _conn: &mut DaemonConn,
) -> Result<()> {
    let msg: Inbound = match serde_json::from_str(json) {
        Ok(m) => m,
        Err(_) => return Ok(()), // ignore unparseable frames
    };

    match msg {
        Inbound::Res(res) => {
            let Some(kind) = state.pending.remove(&res.req_id) else {
                return Ok(());
            };
            match kind {
                PendingKind::Subscribe => {} // subscription is set server-side
                PendingKind::AgentList => {
                    if res.ok {
                        if let Some(data) = res.data {
                            state.agents = parse_agent_list(&data);
                        }
                    }
                }
                PendingKind::AgentSpawn => {
                    if res.ok {
                        state.set_status("Agent spawned");
                    } else if let Some(err) = res.error {
                        state.set_status(format!("Spawn failed: {}", err.message));
                    }
                }
            }
        }
        Inbound::Evt(evt) => handle_event_envelope(evt, state),
        Inbound::Fatal(f) => {
            return Err(anyhow::anyhow!("daemon fatal: {} — {}", f.code, f.message));
        }
        Inbound::HelloAck(_) => {} // shouldn't arrive post-handshake
    }
    Ok(())
}

fn handle_event_envelope(evt: crate::daemon::wire::Evt, state: &mut AppState) {
    match evt.topic.as_str() {
        "agent.created" => {
            if let Ok(snap) =
                serde_json::from_value::<crate::agents::AgentSnapshot>(evt.data.clone())
            {
                if !state.agents.iter().any(|a| a.id() == snap.id) {
                    state.agents.push(AgentState::new(snap));
                }
            }
        }
        "agent.exited" => {
            if let Some(id) = evt.data.get("id").and_then(|v| v.as_i64()) {
                let id = id as i32;
                if let Some(a) = state.find_agent_mut(id) {
                    a.status = AgentStatus::Exited;
                }
            }
        }
        "agent.statusChanged" => {
            if let Some(id) = evt.data.get("id").and_then(|v| v.as_i64()) {
                let id = id as i32;
                if let Some(a) = state.find_agent_mut(id) {
                    let status_str =
                        evt.data.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    a.status = match status_str {
                        "active" => {
                            let tool = evt
                                .data
                                .get("tool")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            AgentStatus::Active(tool)
                        }
                        "waiting" => AgentStatus::Waiting,
                        "exited" => AgentStatus::Exited,
                        _ => AgentStatus::Idle,
                    };
                }
            }
        }
        _ => {}
    }
}

#[derive(PartialEq)]
enum AppAction {
    Continue,
    Quit,
}

async fn handle_event(
    event: Event,
    state: &mut AppState,
    conn: Option<&mut DaemonConn>,
) -> Result<AppAction> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if let Some(action) = state.keymap.matches(key.modifiers, key.code) {
                match action {
                    Action::Quit => return Ok(AppAction::Quit),
                    Action::ToggleLayout => {
                        state.focus = match &state.focus {
                            FocusMode::Editor => FocusMode::Office,
                            _ => FocusMode::Editor,
                        };
                    }
                    Action::FocusOffice => {
                        state.focus = FocusMode::Office;
                    }
                }
                return Ok(AppAction::Continue);
            }

            match key.code {
                KeyCode::Char('q') if key.modifiers == KeyModifiers::NONE => {
                    if matches!(state.focus, FocusMode::Office | FocusMode::Editor) {
                        return Ok(AppAction::Quit);
                    }
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(AppAction::Quit);
                }
                KeyCode::Tab => {
                    match tab_press(&state.focus, &state.agent_ids()) {
                        TabOutcome::Focus(new) => state.focus = new,
                        TabOutcome::PassThroughToPty => {
                            // PTY input wired in Phase 4
                        }
                        TabOutcome::NoOp => {}
                    }
                }
                _ => {}
            }
        }

        Event::Mouse(mouse) => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if let Some(action) = chrome::hit_test(&state.hit_rects, mouse.column, mouse.row) {
                    match action {
                        ChromeAction::SpawnAgent => {
                            if let Some(c) = conn {
                                send_agent_spawn(c, state).await?;
                            } else {
                                state.set_status("Not connected to daemon");
                            }
                        }
                        ChromeAction::ToggleLayout => {
                            state.focus = match &state.focus {
                                FocusMode::Editor => FocusMode::Office,
                                _ => FocusMode::Editor,
                            };
                        }
                        ChromeAction::OpenSettings => {
                            state.focus = FocusMode::Modal;
                        }
                        ChromeAction::ZoomIn => state.zoom_in(),
                        ChromeAction::ZoomOut => state.zoom_out(),
                    }
                }
            }
        }

        Event::Paste(text) => {
            if state.focus.is_pty_agent() {
                // PTY input wired in Phase 4
                let _ = text;
            }
        }

        _ => {}
    }
    Ok(AppAction::Continue)
}

fn render(frame: &mut ratatui::Frame, state: &mut AppState) {
    let areas = chrome::split_areas(frame.area());
    draw_main(frame, areas.main, state);
    state.hit_rects = chrome::draw(frame, &state.focus, state.zoom, state.agents.len());
}

fn draw_main(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    // Reconnect overlay takes over the whole main area
    if let Some(ref rs) = state.reconnect {
        let attempt_str = if rs.attempt == 0 {
            "Connecting…".to_string()
        } else {
            format!("Reconnecting… (attempt {}, {}s elapsed)", rs.attempt, rs.elapsed_secs())
        };
        let lines = vec![
            Line::raw(""),
            Line::styled(&attempt_str, Style::default().fg(Color::Yellow)),
            Line::raw(""),
            Line::styled("  Ctrl+C to quit", Style::default().fg(Color::DarkGray)),
        ];
        let p = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Pixel Agents"))
            .style(Style::default());
        frame.render_widget(p, area);
        return;
    }

    if state.agents.is_empty() {
        let msg = if let Some(ref s) = state.status {
            s.text.as_str()
        } else {
            "No agents — press [ + Agent ] or Tab to spawn one"
        };
        let p = Paragraph::new(msg)
            .block(Block::default().borders(Borders::ALL).title("Pixel Agents"))
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, area);
        return;
    }

    let lines: Vec<Line> = state
        .agents
        .iter()
        .map(|a| {
            let focused = matches!(&state.focus, FocusMode::PtyAgent(id) if *id == a.id());
            let indicator = if focused { "▶ " } else { "  " };
            let status_label = a.status.label();
            let status_color = match &a.status {
                AgentStatus::Active(_) => Color::Green,
                AgentStatus::Waiting => Color::Yellow,
                AgentStatus::Exited => Color::DarkGray,
                AgentStatus::Idle => Color::Cyan,
            };
            let tool_suffix = match &a.status {
                AgentStatus::Active(tool) if !tool.is_empty() => format!(" ({})", tool),
                _ => String::new(),
            };
            Line::from(vec![
                Span::raw(indicator),
                Span::styled(
                    format!("Agent #{}", a.id()),
                    Style::default().fg(if focused { Color::White } else { Color::Gray }),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("[{}{}]", status_label, tool_suffix),
                    Style::default().fg(status_color),
                ),
                Span::raw(format!("  session:{:.8}", a.snapshot.session_id)),
            ])
        })
        .collect();

    let status_text = if let Some(ref s) = state.status {
        format!(" {} ", s.text)
    } else {
        format!(
            " {} agent{} ",
            state.agents.len(),
            if state.agents.len() == 1 { "" } else { "s" }
        )
    };

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Office — {}", state.focus.label()))
                .title_bottom(status_text),
        )
        .style(Style::default().fg(Color::White));
    frame.render_widget(p, area);
}

// ── SIGWINCH abstraction ──────────────────────────────────────────────────────

#[cfg(unix)]
struct Sigwinch(tokio::signal::unix::Signal);

#[cfg(unix)]
impl Sigwinch {
    fn new() -> Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        Ok(Self(signal(SignalKind::window_change())?))
    }
    async fn recv(&mut self) {
        self.0.recv().await;
    }
}

#[cfg(not(unix))]
struct Sigwinch;

#[cfg(not(unix))]
impl Sigwinch {
    fn new() -> Result<Self> {
        Ok(Self)
    }
    async fn recv(&mut self) {
        std::future::pending::<()>().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::Keymap;

    fn make_state() -> AppState {
        AppState::new(Keymap::load())
    }

    #[test]
    fn zoom_clamps_at_10() {
        let mut s = make_state();
        s.zoom = 10;
        s.zoom_in();
        assert_eq!(s.zoom, 10);
    }

    #[test]
    fn zoom_clamps_at_1() {
        let mut s = make_state();
        s.zoom = 1;
        s.zoom_out();
        assert_eq!(s.zoom, 1);
    }

    #[test]
    fn zoom_increments() {
        let mut s = make_state();
        s.zoom = 3;
        s.zoom_in();
        assert_eq!(s.zoom, 4);
    }

    #[test]
    fn req_ids_are_unique_and_monotonic() {
        let mut s = make_state();
        let a = s.next_req_id();
        let b = s.next_req_id();
        let c = s.next_req_id();
        assert!(a < b && b < c);
    }

    #[test]
    fn pending_map_roundtrip() {
        let mut s = make_state();
        let id = s.next_req_id();
        s.pending.insert(id, PendingKind::AgentList);
        assert_eq!(s.pending.remove(&id), Some(PendingKind::AgentList));
        assert!(s.pending.is_empty());
    }

    #[test]
    fn agent_list_event_populates_state() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{
                "id": 1,
                "sessionId": "test-session",
                "palette": 0,
                "hueShift": 0
            }]
        });
        s.agents = parse_agent_list(&data);
        assert_eq!(s.agents.len(), 1);
        assert_eq!(s.agents[0].id(), 1);
    }

    #[test]
    fn handle_agent_created_event_adds_agent() {
        let mut s = make_state();
        let evt = crate::daemon::wire::Evt {
            topic: "agent.created".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({
                "id": 2,
                "sessionId": "new-session",
                "palette": 1,
                "hueShift": 0
            }),
        };
        handle_event_envelope(evt, &mut s);
        assert_eq!(s.agents.len(), 1);
        assert_eq!(s.agents[0].id(), 2);
    }

    #[test]
    fn handle_agent_created_deduplicates() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{"id": 3, "sessionId": "s", "palette": 0, "hueShift": 0}]
        });
        s.agents = parse_agent_list(&data);

        let evt = crate::daemon::wire::Evt {
            topic: "agent.created".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({ "id": 3, "sessionId": "s", "palette": 0, "hueShift": 0 }),
        };
        handle_event_envelope(evt, &mut s);
        assert_eq!(s.agents.len(), 1);
    }

    #[test]
    fn handle_agent_exited_marks_exited() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{"id": 4, "sessionId": "s", "palette": 0, "hueShift": 0}]
        });
        s.agents = parse_agent_list(&data);

        let evt = crate::daemon::wire::Evt {
            topic: "agent.exited".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({ "id": 4 }),
        };
        handle_event_envelope(evt, &mut s);
        assert_eq!(s.agents[0].status, AgentStatus::Exited);
    }

    #[test]
    fn handle_status_changed_active() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{"id": 5, "sessionId": "s", "palette": 0, "hueShift": 0}]
        });
        s.agents = parse_agent_list(&data);

        let evt = crate::daemon::wire::Evt {
            topic: "agent.statusChanged".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({ "id": 5, "status": "active", "tool": "Bash" }),
        };
        handle_event_envelope(evt, &mut s);
        assert_eq!(s.agents[0].status, AgentStatus::Active("Bash".into()));
    }

    #[test]
    fn on_disconnected_clears_pending_and_sets_reconnect() {
        let mut s = make_state();
        s.pending.insert(1, PendingKind::AgentList);
        s.on_disconnected();
        assert!(s.pending.is_empty());
        assert!(s.reconnect.is_some());
    }

    #[test]
    fn on_connected_clears_reconnect() {
        let mut s = make_state();
        s.reconnect = Some(ReconnectState::new());
        s.on_connected("boot-1".into());
        assert!(s.reconnect.is_none());
        assert_eq!(s.connected_boot_id.as_deref(), Some("boot-1"));
    }

    #[test]
    fn on_connected_new_boot_id_clears_agents() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{"id": 1, "sessionId": "s", "palette": 0, "hueShift": 0}]
        });
        s.agents = parse_agent_list(&data);
        s.on_connected("boot-1".into()); // first connect — no clear
        assert_eq!(s.agents.len(), 1);

        s.on_connected("boot-2".into()); // bootId changed — clear
        assert!(s.agents.is_empty());
    }

    #[test]
    fn on_connected_same_boot_id_keeps_agents() {
        let mut s = make_state();
        let data = serde_json::json!({
            "agents": [{"id": 1, "sessionId": "s", "palette": 0, "hueShift": 0}]
        });
        s.agents = parse_agent_list(&data);
        s.on_connected("boot-1".into());
        s.on_connected("boot-1".into()); // same bootId — keep agents
        assert_eq!(s.agents.len(), 1);
    }

    #[test]
    fn on_connected_boot_id_change_resets_pty_focus() {
        let mut s = make_state();
        s.focus = FocusMode::PtyAgent(3);
        s.on_connected("boot-1".into()); // first connect
        s.on_connected("boot-2".into()); // bootId changed
        assert_eq!(s.focus, FocusMode::Office);
    }
}
