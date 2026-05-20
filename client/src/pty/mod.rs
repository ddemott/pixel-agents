//! PTY hosting client side (Phase 4 — not yet implemented).
//!
//! This module currently exists only to pin the terminal-emulator dependency
//! sourcing decision: `wezterm-term` is unpublished, so we depend on the Tattoy
//! project's published fork (`tattoy-wezterm-term`). The compile-time smoke
//! below proves the crate resolves and exposes the API Phase 4 relies on
//! (`Terminal::advance_bytes`) under the expected `tattoy_wezterm_term` path.
//!
//! Phase 4 work (per `docs/tui-implementation-plan.md` §6): per-agent `Terminal`
//! fed via `PtyByteTap`, scrollback, input/resize/paste/mouse.

#[cfg(test)]
mod tests {
    /// Compile-time only: confirms `tattoy_wezterm_term::Terminal::advance_bytes`
    /// exists with the signature Phase 4 integration expects. Never invoked at
    /// runtime — the `Terminal` constructor needs a full config we don't build
    /// here; naming the type + method type-checks the API and catches a renamed
    /// fork at `cargo test` time.
    fn api_smoke(term: &mut tattoy_wezterm_term::Terminal) {
        term.advance_bytes(b"");
    }

    #[test]
    fn dependency_resolves() {
        // Referencing `api_smoke` (which type-checks the forked terminal API)
        // proves the dependency sourcing is resolved and the API path holds.
        let _ = api_smoke;
    }
}
