use anyhow::Result;
use futures_util::StreamExt;
use ratatui::crossterm::event::{Event, EventStream, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::time::{interval, Duration};

use crate::daemon::DaemonConn;
use crate::tui::Tui;

const FRAME_INTERVAL: Duration = Duration::from_millis(17); // ~60 fps

pub async fn run(mut conn: DaemonConn) -> Result<()> {
    let mut tui = Tui::new()?;
    let mut events = EventStream::new();
    let mut tick = interval(FRAME_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sigwinch = Sigwinch::new()?;

    loop {
        tokio::select! {
            // Daemon message
            frame = conn.recv_frame() => {
                match frame {
                    Ok(_frame) => {
                        // Phase 2 Day 8: daemon frames dispatched in Day 9+
                    }
                    Err(e) => {
                        drop(tui);
                        return Err(e.context("daemon disconnected"));
                    }
                }
            }

            // Crossterm input / resize events
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        if handle_event(event) == AppAction::Quit {
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

            // SIGWINCH — ensures resize is caught on terminals that don't send
            // crossterm Resize events inline; no-op future on non-unix.
            _ = sigwinch.recv() => {
                tui.draw(render)?;
            }

            // Frame tick — render at ~60 fps
            _ = tick.tick() => {
                tui.draw(render)?;
            }
        }
    }
}

#[derive(PartialEq)]
enum AppAction {
    Continue,
    Quit,
}

fn handle_event(event: Event) -> AppAction {
    match event {
        Event::Key(key) if key.kind == ratatui::crossterm::event::KeyEventKind::Press => {
            match key.code {
                KeyCode::Char('q') => return AppAction::Quit,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return AppAction::Quit
                }
                _ => {}
            }
        }
        _ => {}
    }
    AppAction::Continue
}

fn render(frame: &mut ratatui::Frame) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    // Main area placeholder
    let main = Paragraph::new("Pixel Agents TUI — Day 8 skeleton")
        .block(Block::default().borders(Borders::ALL).title("Office"))
        .style(Style::default().fg(Color::White));
    frame.render_widget(main, chunks[0]);

    // Bottom toolbar placeholder
    let toolbar = Paragraph::new(Line::from(vec![
        Span::styled(" + Agent ", Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(" Layout ", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw("  "),
        Span::styled(" Settings ", Style::default().fg(Color::Black).bg(Color::Blue)),
        Span::raw("    "),
        Span::styled("q / Ctrl-C: quit", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(toolbar, chunks[1]);
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
    fn new() -> Result<Self> { Ok(Self) }
    async fn recv(&mut self) {
        std::future::pending::<()>().await
    }
}
