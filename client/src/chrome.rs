// Bottom toolbar + top status bar chrome.
//
// draw() renders the chrome onto the frame and returns a HitMap so the input
// handler can resolve mouse clicks to actions.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::focus::FocusMode;

/// Client-chrome actions triggered by mouse clicks or keyboard shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeAction {
    SpawnAgent,
    ToggleLayout,
    OpenSettings,
    ZoomIn,
    ZoomOut,
}

/// A screen region associated with a chrome action.
#[derive(Debug, Clone)]
pub struct HitRect {
    pub rect: Rect,
    pub action: ChromeAction,
}

/// Hit-test a mouse event against a list of hit rects. Returns the action for
/// the first rect containing `(col, row)`, or `None` if no match.
pub fn hit_test(hits: &[HitRect], col: u16, row: u16) -> Option<ChromeAction> {
    hits.iter().find_map(|h| {
        if col >= h.rect.x
            && col < h.rect.x + h.rect.width
            && row >= h.rect.y
            && row < h.rect.y + h.rect.height
        {
            Some(h.action)
        } else {
            None
        }
    })
}

/// Layout result from `split_areas`: the three main regions.
pub struct Areas {
    pub status_bar: Rect,
    pub main: Rect,
    pub toolbar: Rect,
}

/// Split the full terminal area into status_bar / main / toolbar.
pub fn split_areas(area: Rect) -> Areas {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // status bar (top)
            Constraint::Min(0),    // main content
            Constraint::Length(1), // bottom toolbar
        ])
        .split(area);
    Areas { status_bar: chunks[0], main: chunks[1], toolbar: chunks[2] }
}

/// Draw chrome (status bar + toolbar) onto the frame.
/// Returns hit rects for interactive elements.
pub fn draw(frame: &mut Frame, focus: &FocusMode, zoom: u8, agent_count: usize) -> Vec<HitRect> {
    let areas = split_areas(frame.area());
    let mut hits = Vec::new();

    draw_status_bar(frame, areas.status_bar, focus, zoom, &mut hits);
    draw_toolbar(frame, areas.toolbar, agent_count, &mut hits);

    hits
}

fn draw_status_bar(
    frame: &mut Frame,
    area: Rect,
    focus: &FocusMode,
    zoom: u8,
    hits: &mut Vec<HitRect>,
) {
    // Left: focus label
    let focus_label = format!(" {} ", focus.label());

    // Right: zoom controls  [-] 2x [+]
    let zoom_str = format!("{}x", zoom);
    let zoom_minus = " [-] ";
    let zoom_plus = " [+] ";
    let zoom_right_width = (zoom_minus.len() + zoom_str.len() + zoom_plus.len()) as u16;

    // Build status bar as two chunks: left label, right zoom controls
    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(zoom_right_width),
        ])
        .split(area);

    // Focus mode label (left)
    let focus_style = focus_style(focus);
    let status_left = Paragraph::new(Line::from(vec![
        Span::styled(&focus_label, focus_style),
    ]));
    frame.render_widget(status_left, status_chunks[0]);

    // Zoom controls (right)
    let right_area = status_chunks[1];

    // Build zoom control spans with hit rects
    let minus_w = zoom_minus.len() as u16;
    let zoom_label_w = zoom_str.len() as u16;
    let plus_w = zoom_plus.len() as u16;

    let minus_rect = Rect {
        x: right_area.x,
        y: right_area.y,
        width: minus_w,
        height: 1,
    };
    let plus_rect = Rect {
        x: right_area.x + minus_w + zoom_label_w,
        y: right_area.y,
        width: plus_w,
        height: 1,
    };

    hits.push(HitRect { rect: minus_rect, action: ChromeAction::ZoomOut });
    hits.push(HitRect { rect: plus_rect, action: ChromeAction::ZoomIn });

    let zoom_line = Paragraph::new(Line::from(vec![
        Span::styled(zoom_minus, Style::default().fg(Color::Cyan)),
        Span::styled(zoom_str, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(zoom_plus, Style::default().fg(Color::Cyan)),
    ]));
    frame.render_widget(zoom_line, right_area);
}

fn draw_toolbar(frame: &mut Frame, area: Rect, agent_count: usize, hits: &mut Vec<HitRect>) {
    let btn_agent = " + Agent ";
    let btn_layout = " Layout ";
    let btn_settings = " Settings ";
    let sep = "  ";
    let agent_w = btn_agent.len() as u16;
    let layout_w = btn_layout.len() as u16;
    let settings_w = btn_settings.len() as u16;
    let sep_w = sep.len() as u16;

    // Buttons start at x=0
    let x0 = area.x;
    let agent_rect =
        Rect { x: x0, y: area.y, width: agent_w, height: 1 };
    let layout_rect =
        Rect { x: x0 + agent_w + sep_w, y: area.y, width: layout_w, height: 1 };
    let settings_rect =
        Rect { x: x0 + agent_w + sep_w + layout_w + sep_w, y: area.y, width: settings_w, height: 1 };

    hits.push(HitRect { rect: agent_rect, action: ChromeAction::SpawnAgent });
    hits.push(HitRect { rect: layout_rect, action: ChromeAction::ToggleLayout });
    hits.push(HitRect { rect: settings_rect, action: ChromeAction::OpenSettings });

    // Help text (right side)
    let help = "  q / Ctrl-C: quit";
    let agent_count_str = if agent_count > 0 {
        format!("  {} agent{}", agent_count, if agent_count == 1 { "" } else { "s" })
    } else {
        String::new()
    };

    let toolbar_line = Paragraph::new(Line::from(vec![
        Span::styled(
            btn_agent,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(sep),
        Span::styled(btn_layout, Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw(sep),
        Span::styled(btn_settings, Style::default().fg(Color::Black).bg(Color::Blue)),
        Span::styled(&agent_count_str, Style::default().fg(Color::DarkGray)),
        Span::styled(help, Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(toolbar_line, area);
}

fn focus_style(focus: &FocusMode) -> Style {
    match focus {
        FocusMode::Office => Style::default().fg(Color::White).bg(Color::DarkGray),
        FocusMode::PtyAgent(_) => Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD),
        FocusMode::Editor => Style::default().fg(Color::Black).bg(Color::Yellow),
        FocusMode::Modal => Style::default().fg(Color::Black).bg(Color::Magenta),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_inside_rect() {
        let hits = vec![HitRect {
            rect: Rect { x: 0, y: 10, width: 9, height: 1 },
            action: ChromeAction::SpawnAgent,
        }];
        assert_eq!(hit_test(&hits, 4, 10), Some(ChromeAction::SpawnAgent));
    }

    #[test]
    fn hit_test_outside_rect() {
        let hits = vec![HitRect {
            rect: Rect { x: 0, y: 10, width: 9, height: 1 },
            action: ChromeAction::SpawnAgent,
        }];
        assert_eq!(hit_test(&hits, 4, 9), None);  // wrong row
        assert_eq!(hit_test(&hits, 9, 10), None);  // past right edge
    }

    #[test]
    fn hit_test_first_match_wins() {
        let hits = vec![
            HitRect { rect: Rect { x: 0, y: 0, width: 10, height: 1 }, action: ChromeAction::SpawnAgent },
            HitRect { rect: Rect { x: 0, y: 0, width: 10, height: 1 }, action: ChromeAction::ZoomIn },
        ];
        assert_eq!(hit_test(&hits, 5, 0), Some(ChromeAction::SpawnAgent));
    }

    #[test]
    fn hit_test_empty_list() {
        assert_eq!(hit_test(&[], 0, 0), None);
    }

    #[test]
    fn split_areas_toolbar_is_last_row() {
        let area = Rect { x: 0, y: 0, width: 80, height: 24 };
        let areas = split_areas(area);
        assert_eq!(areas.toolbar.y, 23);
        assert_eq!(areas.toolbar.height, 1);
    }

    #[test]
    fn split_areas_status_bar_is_first_row() {
        let area = Rect { x: 0, y: 0, width: 80, height: 24 };
        let areas = split_areas(area);
        assert_eq!(areas.status_bar.y, 0);
        assert_eq!(areas.status_bar.height, 1);
    }

    #[test]
    fn split_areas_main_fills_middle() {
        let area = Rect { x: 0, y: 0, width: 80, height: 24 };
        let areas = split_areas(area);
        assert_eq!(areas.main.y, 1);
        assert_eq!(areas.main.height, 22); // 24 - 2
    }
}
