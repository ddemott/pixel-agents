/// Client focus mode — determines key routing and which pane is active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusMode {
    /// Office canvas is active; keys drive camera/selection.
    Office,
    /// A specific agent's PTY pane is active; keys are forwarded to PTY
    /// (except reserved Ctrl+Alt combos). Positive id = real agent; negative
    /// id = sub-agent (click focuses parent — handled by `normalize_focus`).
    PtyAgent(i32),
    /// Layout editor is active.
    Editor,
    /// A modal dialog is overlaid; Esc dismisses.
    Modal,
}

impl FocusMode {
    pub fn label(&self) -> &str {
        match self {
            FocusMode::Office => "OFFICE",
            FocusMode::PtyAgent(_) => "AGENT",
            FocusMode::Editor => "EDITOR",
            FocusMode::Modal => "MODAL",
        }
    }

    pub fn is_pty_agent(&self) -> bool {
        matches!(self, FocusMode::PtyAgent(_))
    }

    /// If this is a sub-agent (negative id), return parent focus; else unchanged.
    pub fn normalize(self, parent_of: impl Fn(i32) -> Option<i32>) -> FocusMode {
        match &self {
            FocusMode::PtyAgent(id) if *id < 0 => {
                if let Some(parent) = parent_of(*id) {
                    FocusMode::PtyAgent(parent)
                } else {
                    FocusMode::Office
                }
            }
            _ => self,
        }
    }
}

/// Compute next focus on Tab press given current focus + available agent ids.
///
/// In Office mode: cycle to first agent if any exist; else no-op.
/// In PtyAgent mode: Tab is sent to PTY (caller must handle) — return None to
///   signal "pass-through".
/// In Editor / Modal: no-op.
pub fn tab_press(current: &FocusMode, agents: &[i32]) -> TabOutcome {
    match current {
        FocusMode::Office => {
            if let Some(&first) = agents.iter().find(|&&id| id > 0) {
                TabOutcome::Focus(FocusMode::PtyAgent(first))
            } else {
                TabOutcome::NoOp
            }
        }
        FocusMode::PtyAgent(_) => TabOutcome::PassThroughToPty,
        FocusMode::Editor | FocusMode::Modal => TabOutcome::NoOp,
    }
}

#[derive(Debug, PartialEq)]
pub enum TabOutcome {
    Focus(FocusMode),
    PassThroughToPty,
    NoOp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_label() {
        assert_eq!(FocusMode::Office.label(), "OFFICE");
    }

    #[test]
    fn pty_agent_label() {
        assert_eq!(FocusMode::PtyAgent(3).label(), "AGENT");
    }

    #[test]
    fn tab_office_no_agents_noop() {
        assert_eq!(tab_press(&FocusMode::Office, &[]), TabOutcome::NoOp);
    }

    #[test]
    fn tab_office_with_agents_focuses_first() {
        let result = tab_press(&FocusMode::Office, &[7]);
        assert_eq!(result, TabOutcome::Focus(FocusMode::PtyAgent(7)));
    }

    #[test]
    fn tab_office_skips_subagents() {
        // negative IDs are sub-agents; only positive IDs should be focusable via Tab
        let result = tab_press(&FocusMode::Office, &[-1, -2, 5]);
        assert_eq!(result, TabOutcome::Focus(FocusMode::PtyAgent(5)));
    }

    #[test]
    fn tab_in_pty_mode_passthrough() {
        assert_eq!(
            tab_press(&FocusMode::PtyAgent(3), &[3]),
            TabOutcome::PassThroughToPty
        );
    }

    #[test]
    fn tab_in_editor_noop() {
        assert_eq!(tab_press(&FocusMode::Editor, &[1, 2]), TabOutcome::NoOp);
    }

    #[test]
    fn normalize_subagent_to_parent() {
        let f = FocusMode::PtyAgent(-1);
        let normalized = f.normalize(|id| if id == -1 { Some(5) } else { None });
        assert_eq!(normalized, FocusMode::PtyAgent(5));
    }

    #[test]
    fn normalize_unknown_subagent_falls_back_to_office() {
        let f = FocusMode::PtyAgent(-99);
        let normalized = f.normalize(|_| None);
        assert_eq!(normalized, FocusMode::Office);
    }

    #[test]
    fn normalize_real_agent_unchanged() {
        let f = FocusMode::PtyAgent(3);
        let normalized = f.clone().normalize(|_| None);
        assert_eq!(normalized, f);
    }
}
