# Critique Round 1 — TUI Architecture v1

## Summary

- **5 critical issues** (block shipping or contain MVP parity holes / correctness bugs)
- **12 major issues** (significant gaps; weak justifications; "✓" claims that are really ⚠)
- **9 minor issues** (cosmetic, hygiene, future-iteration)
- **Overall verdict: needs major rework**, primarily on (a) the daemon-authority game-loop decision, (b) the alacritty_terminal embedding strategy, (c) the "port verbatim" claims that aren't, and (d) several MVP items that are actually unaddressed.

---

## Critical Issues

### CRIT-1: alacritty_terminal cannot be used as advertised

**Where:** §9, §6.
**Issue:** `alacritty_terminal` is internal/unstable; the doc's code sample doesn't compile; passthrough of image escapes conflicts with grid parsing.
**Why it matters:** B1–B4 MVP items depend on this. Image-emitting tools (web fetch, future Kitty graphics from claude) silently corrupted.
**Proposed fix:** Either dual-parse (raw-byte tap + grid parser) with a pinned alacritty_terminal vendored fork; or switch to `wezterm-term`/`vt100-rust` with documented limitations. Fix the code sample.

### CRIT-2: A4 "Restore agents on relaunch" is ⚠ on an MVP item

**Where:** §16, §21.
**Issue:** Checklist exit criterion requires every MVP item ✓. Architecture admits ⚠. This is a regression from current VS Code behavior where VS Code keeps terminals alive across reloads.
**Proposed fix:** Either implement `--resume` auto-respawn on daemon boot (spec session-JSONL liveness checks and failure paths), or commit to OS supervisor (systemd-user/launchd/Scheduled Task) so daemon auto-restarts.

### CRIT-3: Daemon-authoritative game-loop FSM breaks animation fidelity

**Where:** §5.
**Issue:** 30 Hz snapshot + client interpolation breaks F6 pixel-perfect; per-frame animation phase needs 60 Hz or client-derived; bandwidth math hand-waved; multi-client editor state vs daemon authority contradicts itself.
**Proposed fix:** Move FSM/animation to clients; daemon broadcasts events + world model only; clients run deterministic OfficeState from event stream + shared seed. Matches the existing engine code which is already pure.

### CRIT-4: "Port verbatim" is fiction — 70+ vscode references across the four reused files

**Where:** §11.
**Issue:** Grep shows 26/23/18/10 vscode references across fileWatcher/agentManager/transcriptParser/timerManager — including `vscode.window.terminals` adoption (the entire C7 mechanism). Every function signature carries `webview: vscode.Webview | undefined`.
**Proposed fix:** Phase-0 prerequisite: extract `MessageSender`/`AgentEventSink` interface, refactor the four files behind it inside the existing extension, ship through CI. Only then begin TUI port. Spec the interface signature in the doc.

### CRIT-5: server.json multi-process arbitration unsound

**Where:** §15, §4.
**Issue:** Doc unilaterally changes "reuse" behavior to "exit"; PID liveness check is racy; 2s timeout arbitrary; concurrent VS Code extension + TUI daemon both want the same file; token rotation invalidates active clients.
**Proposed fix:** Pick a resolution rule (cooperative, separate files, or shared daemon). Add `bootId` UUID. Spec the cold-start retry path.

---

## Major Issues (summary — full detail in original critique)

- **MAJ-1**: Wire protocol §10 sketched not specified — no schemas for `WorldSnapshot` etc.
- **MAJ-2**: 4MB JSON lines with base64-PTY is wrong; use length-prefixed binary multiplexed channel.
- **MAJ-3**: Capability detection §7 races against user input on stdin; pre-app input queue needed; allow `PIXEL_AGENTS_TIER` override.
- **MAJ-4**: Kitty unicode placeholders only really work in Kitty; WezTerm/Ghostty/Konsole have rough edges; tmux passthrough breaks placeholders.
- **MAJ-5**: Sixel T3 30fps target unrealistic on xterm; target should be tier-specific.
- **MAJ-6**: PTY resize "smaller wins" is wrong — should follow focused client; debounce focus storms.
- **MAJ-7**: Linux audio fallback cascade misses `pw-play` (PipeWire); terminal bell defeats cross-window notification — add `notify-send` etc.
- **MAJ-8**: Asset cache budget math doesn't reconcile across daemon/client caps.
- **MAJ-9**: Bracketed paste + mouse capture passthrough to PTY unspecified.
- **MAJ-10**: Auto-update is security/reliability hazard; opt-in only, no in-place self-upgrade.
- **MAJ-11**: Layout file conflict resolution racey; embed writer-tag in file.
- **MAJ-12**: F6 pixel-perfect is false for half-block tiers (1:2 aspect ratio); demote to ⚠ with explicit accepted limitation.

## Minor Issues

- MIN-1: OQ-2 (Kitty placeholders in WezTerm) is blocking, not open
- MIN-2: tmux passthrough probe nesting unspecified
- MIN-3: hook auth token rotation policy
- MIN-4: schema versioning on world.snapshot
- MIN-5: logging format/path undocumented
- MIN-6: Tab focus toggle conflicts with claude tab-complete
- MIN-7: keymap customization not provided
- MIN-8: citation hygiene (one weak source)
- MIN-9: naming collision (`pixel-agents` vs `pixel-agents-daemon` vs launcher)

## Parity Checklist Audit

- A4 ⚠ on MVP — **blocker**
- F6 ✓ on half-block tiers — **rebuttal**, demote to ⚠
- R3 ✓ achieved by "cutting features until we hit it" — **demote to ⚠ or commit to cuts**
- B1–B4, C1–C7, E1–E6 conditionally confirmed pending CRIT-1 and CRIT-4
- G1–G7 conditionally confirmed pending CRIT-3

## Recommended Focus for Next Iteration

1. Spec the `MessageSender` / `AgentEventSink` interface (CRIT-4).
2. Resolve game-loop authority decisively (CRIT-3).
3. Fix A4 — pick `--resume` or OS supervisor (CRIT-2).
4. Spec wire protocol completely with TS declarations + binary PTY channel (MAJ-1, MAJ-2).
5. Pick PTY parser strategy with passthrough proof (CRIT-1).

Honorable: CRIT-5, MAJ-12, MAJ-10.
