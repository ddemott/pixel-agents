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
use crate::assets::AssetStore;
use crate::chrome::{self, ChromeAction};
use crate::daemon::wire::{ClientCapabilities, Inbound, Req, RenderingCap};
use crate::render::kitty::KittyUploader;
use crate::render::char_sprites::CharSpriteStore;
use crate::render::scene::{compose_cells_into, View};
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
    AssetBlob,
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
    /// Office simulation — initialized from the first HelloAck's WorldSnapshot.
    /// Determinism lives inside `OfficeState`: each agent owns a wander RNG
    /// seeded `worldSeed ^ agentId`, so no client-wide RNG is threaded here.
    office: Option<crate::office::state::OfficeState>,
    /// Decoded sprite store, populated from the daemon's 0x02 asset channel.
    assets: AssetStore,
    /// Character sprite sheets sliced into per-state frames, hue-shifted lazily.
    char_sprites: crate::render::char_sprites::CharSpriteStore,
    /// Tracks Kitty image uploads (T1-K) so each sprite transmits once.
    kitty: KittyUploader,
    /// Active terminal rendering capability (tier). Drives the office draw path.
    render_cap: RenderingCap,
    /// Per-agent headless terminal models, fed by the daemon's 0x01 PTY stream
    /// (keyed by agent id = PTY stream id). Populated for every live agent, not
    /// just the focused one, so a focus switch shows current output immediately.
    terminals: HashMap<i32, crate::pty::PtyTerminal>,
    /// Timestamp of the last rendered frame for dt calculation.
    last_frame: Option<Instant>,
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
            office: None,
            assets: AssetStore::new(),
            char_sprites: crate::render::char_sprites::CharSpriteStore::new(),
            kitty: KittyUploader::new(),
            render_cap: RenderingCap::Truecolor,
            terminals: HashMap::new(),
            last_frame: None,
        }
    }

    /// Feed raw PTY bytes for `agent_id` into its terminal model, creating the
    /// model on first sight (sized to the daemon's spawn default until
    /// resize-follow lands).
    fn ingest_pty(&mut self, agent_id: i32, bytes: &[u8]) {
        self.terminals
            .entry(agent_id)
            .or_insert_with(|| {
                crate::pty::PtyTerminal::new(crate::pty::DEFAULT_COLS, crate::pty::DEFAULT_ROWS)
            })
            .advance(bytes);
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
    state.render_cap = caps.rendering.clone();
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
                        let world_seed = new_conn.world.get("worldSeed")
                            .and_then(|v| v.as_u64()).unwrap_or(0);
                        let catalog = crate::office::catalog::FurnitureCatalog::from_wire(&new_conn.world);
                        let layout = new_conn.world.get("layout")
                            .and_then(|v| crate::office::layout::parse_layout(v))
                            .unwrap_or_else(|| crate::office::types::OfficeLayout::empty(20, 11));
                        state.office = Some(crate::office::state::OfficeState::new(
                            catalog,
                            layout,
                            world_seed as u32,
                        ));
                        conn = Some(new_conn);
                        state.on_connected(boot_id);
                        let c = conn.as_mut().unwrap();
                        send_subscribe(c, &mut state).await?;
                        send_agent_list(c, &mut state).await?;
                        request_assets(c, &mut state).await?;
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
                    Ok(Frame::Asset { asset_id, tier, is_final, bytes }) => {
                        // Decode failures skip the one sprite; the loop keeps running.
                        if let Ok(Some(id)) = state.assets.on_frame(asset_id, tier, is_final, &bytes) {
                            // A `char_N` sheet just completed → hand it to the
                            // character sprite store for slicing.
                            if let Some(palette) = CharSpriteStore::palette_of(&id) {
                                if let Some(sheet) = state.assets.get(&id) {
                                    state.char_sprites.ingest(palette, sheet.clone());
                                }
                            }
                        }
                    }
                    Ok(Frame::PtyOut { stream_id, bytes }) => {
                        // stream_id == agent id (daemon `broadcastPty`). Feed
                        // every agent's stream so a focus switch is instant.
                        state.ingest_pty(stream_id as i32, &bytes);
                    }
                    Ok(_) => {} // PtyIn is client→daemon only; never received here.
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
                let now = Instant::now();
                let dt = state.last_frame
                    .map(|t| (now - t).as_secs_f32().min(0.1))
                    .unwrap_or(0.0);
                state.last_frame = Some(now);
                if let Some(ref mut office) = state.office {
                    office.tick(dt);
                }
                // Build the frame sets for every live (palette, hue_shift) combo
                // before drawing (draw reads the store immutably). Collect first
                // to drop the office borrow.
                let combos: Vec<(u8, i32)> = state
                    .office
                    .as_ref()
                    .map(|o| o.characters.values().map(|c| (c.palette, c.hue_shift)).collect())
                    .unwrap_or_default();
                for (palette, hue) in combos {
                    state.char_sprites.ensure(palette, hue);
                }
                // Selection drives the white outline: a focused PTY agent is the
                // selected office character (hover not wired at cell tiers).
                let selected = match state.focus {
                    FocusMode::PtyAgent(id) => Some(id),
                    _ => None,
                };
                if let Some(office) = state.office.as_mut() {
                    office.selected_agent_id = selected;
                }
                // T1-K/T1-O: transmit any newly-decoded sprites once. `a=t` is
                // display-free, so emitting out-of-band of the draw is safe.
                // (Spatial placement/placeholders land with the Day 17 render
                // pipeline; this just primes the terminal's image cache.)
                if matches!(caps.rendering, RenderingCap::KittyK | RenderingCap::KittyO) {
                    let uploads = state.kitty.pending_uploads(&state.assets);
                    if !uploads.is_empty() {
                        use std::io::Write;
                        let mut out = std::io::stdout();
                        let _ = out.write_all(&uploads);
                        let _ = out.flush();
                    }
                }
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

/// Request a sprite PNG for every catalog asset over the 0x02 blob channel.
/// Frames arrive asynchronously and land in `state.assets` via `on_frame`.
async fn request_assets(conn: &mut DaemonConn, state: &mut AppState) -> Result<()> {
    let ids: Vec<String> = match state.office.as_ref() {
        Some(o) => o.catalog.entries.keys().cloned().collect(),
        None => return Ok(()),
    };
    // Furniture catalog assets + the six character sprite sheets (`char_0`..
    // `char_5`). Unknown ids reply `not_found` and are simply never decoded, so
    // requesting all six is safe even if a pack ships fewer.
    let char_ids = (0..crate::office::types::NUM_PALETTES as u8).map(CharSpriteStore::asset_id);
    for id in ids.into_iter().chain(char_ids) {
        state.assets.register_request(&id);
        let req_id = state.next_req_id();
        state.pending.insert(req_id, PendingKind::AssetBlob);
        let req = Req {
            kind: "req",
            req_id,
            method: "assets.requestBlob".into(),
            params: Some(json!({ "assetId": id, "tier": 0 })),
        };
        conn.send(&req).await?;
    }
    Ok(())
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
                PendingKind::AssetBlob => {} // bytes already streamed over 0x02
            }
        }
        Inbound::Evt(evt) => handle_event_envelope(evt, state),
        Inbound::Fatal(f) => {
            return Err(anyhow::anyhow!("daemon fatal: {} — {}", f.code, f.message));
        }
        Inbound::HelloAck(_) => {} // shouldn't arrive post-handshake; world captured at connect()
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
                    // Mirror into the office sim so a character spawns (with the
                    // matrix spawn effect). Palette/hue/seat come from the daemon
                    // so the character matches its assigned skin.
                    if let Some(office) = state.office.as_mut() {
                        office.add_agent(
                            snap.id,
                            Some(snap.palette),
                            Some(snap.hue_shift),
                            snap.seat_id.as_deref(),
                            false,
                        );
                    }
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
                // Despawn matrix effect; the character self-removes at completion.
                if let Some(office) = state.office.as_mut() {
                    office.remove_agent(id);
                }
                // Drop the terminal model (graceful-death output retention is
                // Phase-4 Day 12 scope).
                state.terminals.remove(&id);
            }
        }
        "agent.statusChanged" => {
            if let Some(id) = evt.data.get("id").and_then(|v| v.as_i64()) {
                let id = id as i32;
                let status_str =
                    evt.data.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let tool = evt
                    .data
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(a) = state.find_agent_mut(id) {
                    a.status = match status_str {
                        "active" => AgentStatus::Active(tool.clone()),
                        "waiting" => AgentStatus::Waiting,
                        "exited" => AgentStatus::Exited,
                        _ => AgentStatus::Idle,
                    };
                }
                // Drive the office FSM + speech bubbles off the same event.
                if let Some(office) = state.office.as_mut() {
                    match status_str {
                        "active" => {
                            office.set_agent_active(id, true);
                            office.set_agent_tool(
                                id,
                                if tool.is_empty() { None } else { Some(tool) },
                            );
                        }
                        "waiting" => {
                            office.set_agent_active(id, false);
                            office.show_waiting_bubble(id);
                        }
                        _ => office.set_agent_active(id, false),
                    }
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

/// Cell-rasterized tiers (no terminal graphics protocol). The half-block
/// rasterizer serves all of them — the terminal down-quantizes the truecolor.
fn is_cell_tier(cap: &RenderingCap) -> bool {
    matches!(
        cap,
        RenderingCap::Truecolor | RenderingCap::C256 | RenderingCap::C16 | RenderingCap::Braille
    )
}

fn render(frame: &mut ratatui::Frame, state: &mut AppState) {
    let areas = chrome::split_areas(frame.area());
    draw_main(frame, areas.main, state);
    state.hit_rects = chrome::draw(frame, &state.focus, state.zoom, state.agents.len());
}

/// Short activity string for the tool overlay (mirrors ToolOverlay's
/// `getActivityLabel`): tool name when active, "Needs approval" when waiting.
fn activity_label(status: &AgentStatus) -> &str {
    match status {
        AgentStatus::Active(tool) if !tool.is_empty() => tool.as_str(),
        AgentStatus::Active(_) => "Working",
        AgentStatus::Waiting => "Needs approval",
        AgentStatus::Exited => "Exited",
        AgentStatus::Idle => "Idle",
    }
}

/// Floating activity label above the selected character (Day 21 tool overlay).
fn draw_tool_overlay(
    buf: &mut ratatui::buffer::Buffer,
    office: &crate::office::state::OfficeState,
    state: &AppState,
    view: &View,
    inner: Rect,
) {
    let Some(id) = office.selected_agent_id else { return };
    let Some(ch) = office.characters.get(&id) else { return };
    if ch.matrix_effect.is_some() {
        return; // hidden during spawn/despawn
    }
    let label = state
        .agents
        .iter()
        .find(|a| a.id() == id)
        .map(|a| activity_label(&a.status))
        .unwrap_or("Idle");
    let text = format!(" {label} ");
    // Above the head: the char sprite top sits ~32 world px above ch.y.
    let (cx, cy) = view.world_to_cell(ch.x, ch.y - 32.0);
    let row = cy - 1;
    let col = cx - text.chars().count() as i32 / 2;
    if row < inner.top() as i32 || row >= inner.bottom() as i32 {
        return;
    }
    let style = Style::default().fg(Color::Black).bg(Color::Rgb(0xee, 0xee, 0xff));
    let x0 = col.max(inner.left() as i32);
    if x0 >= inner.right() as i32 {
        return; // off the right edge — nothing to draw (avoids width underflow)
    }
    buf.set_stringn(
        x0 as u16,
        row as u16,
        &text,
        inner.right() as usize - x0 as usize,
        style,
    );
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

    // Focused agent: its live terminal grid takes over the main area (all
    // tiers). Falls through to the office/list view until the first PTY bytes
    // for that agent arrive (no terminal model yet).
    if let FocusMode::PtyAgent(id) = state.focus {
        if let Some(term) = state.terminals.get(&id) {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!("Agent {id} — {}", state.focus.label()));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            term.render_into(frame.buffer_mut(), inner);
            return;
        }
    }

    // Cell tiers (T4/T5/T6/T6b): render the spatial office into the main area.
    // Image tiers (Kitty/iTerm2/Sixel) keep the agent-list view until the
    // image-tier compositor is wired into the live writer (see render::scene).
    if is_cell_tier(&state.render_cap) {
        if let Some(ref office) = state.office {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!("Office — {}", state.focus.label()));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let view = View::new(office, inner, state.zoom as u16);
            compose_cells_into(frame.buffer_mut(), office, &state.assets, &state.char_sprites, &view);
            draw_tool_overlay(frame.buffer_mut(), office, state, &view, inner);
            return;
        }
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

    /// AppState with a small all-floor office so the event→office bridge has a
    /// sim to mutate.
    fn make_state_with_office() -> AppState {
        use crate::office::catalog::FurnitureCatalog;
        use crate::office::state::OfficeState;
        use crate::office::types::{OfficeLayout, TileType};
        let n = 25;
        let layout = OfficeLayout {
            version: 1,
            cols: 5,
            rows: 5,
            tiles: vec![TileType::Floor1; n],
            furniture: vec![],
            tile_colors: vec![None; n],
        };
        let mut s = make_state();
        s.office = Some(OfficeState::new(FurnitureCatalog::empty(), layout, 1));
        s
    }

    fn created_evt(id: i32, palette: u8) -> crate::daemon::wire::Evt {
        crate::daemon::wire::Evt {
            topic: "agent.created".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({
                "id": id, "sessionId": "s", "palette": palette, "hueShift": 0
            }),
        }
    }

    fn status_evt(id: i32, status: &str, tool: &str) -> crate::daemon::wire::Evt {
        crate::daemon::wire::Evt {
            topic: "agent.statusChanged".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({ "id": id, "status": status, "tool": tool }),
        }
    }

    fn exited_evt(id: i32) -> crate::daemon::wire::Evt {
        crate::daemon::wire::Evt {
            topic: "agent.exited".into(),
            seq: 1,
            ts: 0,
            data: serde_json::json!({ "id": id }),
        }
    }

    #[test]
    fn bridge_created_spawns_office_character() {
        let mut s = make_state_with_office();
        handle_event_envelope(created_evt(7, 2), &mut s);
        let office = s.office.as_ref().unwrap();
        let ch = office.characters.get(&7).expect("character spawned");
        assert_eq!(ch.palette, 2);
        // skip_spawn_effect = false → matrix spawn effect armed.
        assert_eq!(ch.matrix_effect, Some(crate::office::types::MatrixEffectKind::Spawn));
    }

    #[test]
    fn bridge_exited_triggers_despawn_effect() {
        let mut s = make_state_with_office();
        handle_event_envelope(created_evt(7, 0), &mut s);
        // Clear the spawn effect so we observe the despawn transition cleanly.
        s.office.as_mut().unwrap().characters.get_mut(&7).unwrap().matrix_effect = None;
        handle_event_envelope(
            crate::daemon::wire::Evt {
                topic: "agent.exited".into(),
                seq: 2,
                ts: 0,
                data: serde_json::json!({ "id": 7 }),
            },
            &mut s,
        );
        let ch = s.office.as_ref().unwrap().characters.get(&7).expect("still present mid-despawn");
        assert_eq!(ch.matrix_effect, Some(crate::office::types::MatrixEffectKind::Despawn));
    }

    #[test]
    fn bridge_status_active_sets_tool_and_active() {
        let mut s = make_state_with_office();
        handle_event_envelope(created_evt(7, 0), &mut s);
        handle_event_envelope(status_evt(7, "active", "Read"), &mut s);
        let ch = s.office.as_ref().unwrap().characters.get(&7).unwrap();
        assert!(ch.is_active);
        assert_eq!(ch.current_tool.as_deref(), Some("Read"));
    }

    #[test]
    fn bridge_status_waiting_shows_bubble_and_deactivates() {
        let mut s = make_state_with_office();
        handle_event_envelope(created_evt(7, 0), &mut s);
        handle_event_envelope(status_evt(7, "waiting", ""), &mut s);
        let ch = s.office.as_ref().unwrap().characters.get(&7).unwrap();
        assert!(!ch.is_active);
        assert_eq!(ch.bubble_type, Some(crate::office::types::BubbleType::Waiting));
    }

    #[test]
    fn is_cell_tier_classifies_tiers() {
        assert!(is_cell_tier(&RenderingCap::Truecolor));
        assert!(is_cell_tier(&RenderingCap::C256));
        assert!(is_cell_tier(&RenderingCap::Braille));
        assert!(!is_cell_tier(&RenderingCap::KittyK));
        assert!(!is_cell_tier(&RenderingCap::Iterm2));
        assert!(!is_cell_tier(&RenderingCap::Sixel));
    }

    #[test]
    fn render_paints_office_for_cell_tier() {
        use crate::office::catalog::FurnitureCatalog;
        use crate::office::state::OfficeState;
        use crate::office::types::{OfficeLayout, TileType};
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;
        use ratatui::Terminal;

        // All-floor 5×5 office, no reconnect, truecolor tier.
        let layout = OfficeLayout {
            version: 1,
            cols: 5,
            rows: 5,
            tiles: vec![TileType::Floor1; 25],
            furniture: vec![],
            tile_colors: vec![None; 25],
        };
        let mut state = make_state();
        state.render_cap = RenderingCap::Truecolor;
        state.reconnect = None;
        state.office = Some(OfficeState::new(FurnitureCatalog::empty(), layout, 1));

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| render(f, &mut state)).unwrap();

        // Floor cells (FLOOR_BG = Rgb(56,56,74)) were painted into the main area.
        let buf = term.backend().buffer();
        let floor = (0..80u16)
            .flat_map(|x| (0..24u16).map(move |y| (x, y)))
            .any(|(x, y)| buf[(x, y)].bg == Color::Rgb(56, 56, 74));
        assert!(floor, "office floor should render for a cell tier");
    }

    #[test]
    fn render_paints_focused_pty_grid() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        // No office needed: focusing an agent with a live terminal renders its
        // grid over the main area regardless of tier.
        let mut state = make_state();
        state.render_cap = RenderingCap::Truecolor;
        state.reconnect = None;
        state.focus = FocusMode::PtyAgent(3);
        state.ingest_pty(3, b"HELLO");

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| render(f, &mut state)).unwrap();

        // The PTY block title + the ingested text should appear somewhere in
        // the main area (chrome offsets it from buffer row 0, so scan all rows).
        let buf = term.backend().buffer();
        let row_text: String = (0..24u16)
            .flat_map(|y| (0..80u16).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        assert!(row_text.contains("Agent 3"), "PTY block title should render");
        assert!(row_text.contains("HELLO"), "ingested PTY text should render");
    }

    #[test]
    fn focused_pty_falls_back_when_no_terminal_yet() {
        // Focus an agent before any PTY bytes arrive → no terminal model, so the
        // PTY branch is skipped (no panic, falls through to the empty view).
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let mut state = make_state();
        state.reconnect = None;
        state.focus = FocusMode::PtyAgent(9);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| render(f, &mut state)).unwrap();
        assert!(!state.terminals.contains_key(&9));
    }

    #[test]
    fn pty_terminal_dropped_on_agent_exit() {
        let mut s = make_state_with_office();
        handle_event_envelope(created_evt(7, 0), &mut s);
        s.ingest_pty(7, b"x");
        assert!(s.terminals.contains_key(&7));
        handle_event_envelope(exited_evt(7), &mut s);
        assert!(!s.terminals.contains_key(&7), "terminal should drop on exit");
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
