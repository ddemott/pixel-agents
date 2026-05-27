# Pixel Agents — TUI Port Feature Parity Checklist

Source of truth for the TUI port. The design loop exits when every item below has a documented, defensible design in `tui-architecture.md`. Items marked **MVP** must work in the first shippable build; **Full** items can land in later milestones but must be designed up-front.

Status legend (filled by architecture loop):

- ✓ designed and addressed in `tui-architecture.md`
- ⚠ partially addressed or has known limitation accepted
- ✗ not yet addressed

---

## A. Agent Lifecycle

- [ ] A1 (MVP) Spawn agent: equivalent to "+ Agent" — creates a new Claude session with deterministic UUID, opens a child PTY running `claude --session-id <uuid>`, registers the agent.
- [ ] A2 (MVP) Spawn variant: `--dangerously-skip-permissions`.
- [ ] A3 (MVP) Close agent: terminating the underlying claude process despawns its character.
- [ ] A4 (MVP) Restore agents on relaunch from persisted state.
- [ ] A5 (Full) Terminal adoption: detect externally launched `claude` sessions in the workspace and adopt them (project-level 1s scan equivalent).
- [ ] A6 (Full) `/clear` detection — same JSONL becomes a new file; reassign agent to new session.
- [ ] A7 (Full) `/resume` detection via hook SessionStart(source=resume) with grace window.

## B. Terminal / PTY Integration

- [ ] B1 (MVP) Each agent has its own PTY hosted by the daemon, with full stdio bidirectional streaming to the focused client.
- [ ] B2 (MVP) User can type into the focused agent's PTY (interactive `claude`).
- [ ] B3 (MVP) Resize: PTY rows/cols track the visible pane size.
- [ ] B4 (MVP) Scrollback per agent.
- [x] Bx (Phase 4) Mouse protocol forwarding (X10 + SGR) with arbitration — when the PTY has DECSET-grabbed the mouse we emit the correct reports; otherwise client chrome owns the mouse. (Implemented client/src/pty/mod.rs + app.rs; tracked in TODO under Phase 4 slice 6.)
- [ ] B5 (Full) Click-to-focus character → switches the active terminal pane to that agent.
- [ ] B6 (Full) Sub-agent click → focuses parent agent's terminal.
- [ ] B7 (Full) Graceful PTY death: zombie cleanup, status update.

## C. Agent Status Tracking — JSONL & Hooks

- [ ] C1 (MVP) JSONL polling 500ms with partial-line buffering.
- [ ] C2 (MVP) Parse `assistant` records (tool_use, thinking) and `user` records (tool_result, text prompt).
- [ ] C3 (MVP) Parse `system` records with `subtype: "turn_duration"` → reliable turn-end signal.
- [ ] C4 (MVP) Track tool_use lifecycle: start, done (300ms delay to prevent flicker).
- [ ] C5 (Full) Text-idle timer (5s) for text-only turns; suppressed when any tool_use seen in turn.
- [ ] C6 (Full) Permission timer (7s) when non-exempt tool runs without result.
- [ ] C7 (Full) Heuristic mode: 1s main scanner, 3s external scanner, 30s stale check, /clear content detection in first 8KB, dismissal sets (clearDismissedFiles, dismissedJsonlFiles, seededMtimes, pendingClearFiles).
- [ ] C8 (Full) Hooks mode: HTTP server at `~/.pixel-agents/server.json` (port + PID + auth token); receives SessionStart, SessionEnd, Stop, PreToolUse, PostToolUse, PostToolUseFailure, PermissionRequest, Notification, UserPromptSubmit, SubagentStart, SubagentStop.
- [ ] C9 (Full) Per-agent `hookDelivered` flag toggles between modes seamlessly.
- [ ] C10 (Full) Sub-agent permission detection (5s timer on parent → bubbles on both).
- [ ] C11 (Full) `agent_progress`, `bash_progress`, `mcp_progress` JSONL records handled.

## D. Hook Script & Installation

- [ ] D1 (MVP) Hook script bundled as standalone CJS, installed to `~/.pixel-agents/hooks/claude-hook.js`.
- [ ] D2 (MVP) Install/uninstall in `~/.claude/settings.json` (preserves user hooks).
- [ ] D3 (MVP) Server discovery via `~/.pixel-agents/server.json` (port/PID/auth) — multi-window safe.
- [ ] D4 (Full) Settings toggle: enable/disable hooks installation.
- [ ] D5 (Full) Buffer pre-registration events (race between hook arrival and agent registration).

## E. Sub-agents

- [ ] E1 (Full) Detect Task tool sub-agents from JSONL `progress`/`agent_progress` records.
- [ ] E2 (Full) Spawn sub-agent character with negative ID, parent's palette + hueShift.
- [ ] E3 (Full) Spawn at closest free seat to parent (Manhattan distance); fallback to closest walkable.
- [ ] E4 (Full) Click sub-agent → focus parent terminal.
- [ ] E5 (Full) Despawn sub-agent on Task completion.
- [ ] E6 (Full) Teammate detection (named sub-agents like "web-researcher" with spawn flag).

## F. Visual Office Rendering

- [ ] F1 (MVP) Tile-based pixel-art grid (default 20×11, expandable to 64×64).
- [ ] F2 (MVP) Floor tiles with 7 patterns and HSBC colorization.
- [ ] F3 (MVP) Wall tiles with 16-piece auto-tile bitmask, 3D top face, HSBC colorization.
- [ ] F4 (MVP) Furniture rendering with z-sort by Y.
- [ ] F5 (MVP) Characters render with z-sort, 6 palettes + hue shift, 4-directional sprites, left = flipped right.
- [ ] F6 (MVP) Pixel-perfect rendering at integer zoom.
- [ ] F7 (Full) Speech bubbles: permission (amber "..."), waiting (green checkmark, 2s fade).
- [ ] F8 (Full) Tool overlay: activity status label above hovered/selected character.
- [ ] F9 (Full) Selection outline (white) on selected character/furniture.
- [ ] F10 (Full) Matrix-style spawn/despawn effect (0.3s, 16 columns).
- [ ] F11 (Full) Camera follow on agent click; cleared on deselect or manual pan.
- [ ] F12 (Full) Pan via middle-mouse drag (TUI equivalent).
- [ ] F13 (Full) Zoom controls (1×–10× integer).
- [ ] F14 (Full) Grid expansion ghost tiles (dashed outline) outside current grid.

## G. Characters — FSM & AI

- [ ] G1 (MVP) States: idle (wander), active (pathfind to seat, animate by tool type).
- [ ] G2 (MVP) Typing vs reading animation differentiated by tool (Write/Edit/Bash/Task → type; Read/Grep/Glob/WebFetch → read).
- [ ] G3 (MVP) Sitting offset: shift down 6px when in TYPE state.
- [ ] G4 (Full) BFS pathfinding on walkability grid.
- [ ] G5 (Full) Wander AI with wanderLimit before returning to seat.
- [ ] G6 (Full) Chair tiles blocked for non-owners (per-character pathfinding).
- [ ] G7 (Full) Diverse palette assignment via `pickDiversePalette()`; hue shift on 7th+ agent.

## H. Asset System

- [ ] H1 (MVP) Load PNG sprites from `assets/` directory (bundled and user-installed).
- [ ] H2 (MVP) Build furniture catalog from per-folder `manifest.json` files.
- [ ] H3 (MVP) Furniture categories: desks, chairs, storage, electronics, decor, wall, misc.
- [ ] H4 (Full) Rotation groups: front/back/left/right via `groupId`.
- [ ] H5 (Full) State groups: on/off toggle via `groupId` + `orientation`.
- [ ] H6 (Full) Auto-state: electronics swap to ON sprite when active agent faces desk with that item nearby.
- [ ] H7 (Full) Background tiles: top N rows allow other furniture and walk-through.
- [ ] H8 (Full) Surface placement: items can overlap desk tiles.
- [ ] H9 (Full) Wall placement: items pinned to wall tiles.
- [ ] H10 (Full) External asset directories: load from arbitrary paths, override on collision.
- [ ] H11 (Full) Colorize module: dual mode (Photoshop-style colorize vs HSL adjust), per-item color override.
- [ ] H12 (Full) Character sprite loading from `char_0.png`–`char_5.png` (112×96, 7 frames × 3 directions).
- [ ] H13 (Full) Hue-shift on cached sprites for diversity beyond 6 agents.

## I. Layout Editor

- [ ] I1 (Full) Toggle in/out of edit mode.
- [ ] I2 (Full) Tools: SELECT, Floor, Wall, Erase, Furniture place, Furniture pick, Eyedropper.
- [ ] I3 (Full) HSBC color sliders for floor, wall, furniture; Colorize-mode toggle.
- [ ] I4 (Full) Drag-to-paint floor/wall/erase.
- [ ] I5 (Full) Furniture ghost preview with green/red validity.
- [ ] I6 (Full) Place/remove/rotate (R)/toggle-state (T) furniture.
- [ ] I7 (Full) Drag-to-move selected furniture in SELECT.
- [ ] I8 (Full) Delete (red X) and rotate (blue arrow) buttons on selected furniture.
- [ ] I9 (Full) Surface-item priority on click for stacked furniture.
- [ ] I10 (Full) Undo/redo: 50 levels, Ctrl+Z/Y.
- [ ] I11 (Full) Save / Reset buttons; dirty tracking.
- [ ] I12 (Full) Multi-stage Esc to exit nested states.
- [ ] I13 (Full) Erase to VOID (transparent, non-walkable, no furniture).
- [ ] I14 (Full) Grid expansion: click ghost border to grow in any direction; max 64×64.
- [ ] I15 (Full) Character relocation after shrink.

## J. Seats

- [ ] J1 (MVP) Seats derived from chair furniture footprints.
- [ ] J2 (MVP) Multi-tile chair → multiple seats keyed `uid` / `uid:1` / `uid:2`.
- [ ] J3 (Full) Facing direction priority: chair orientation → adjacent desk → forward.
- [ ] J4 (Full) Click character + click seat = reassign.

## K. Persistence

- [ ] K1 (MVP) Layout: `~/.pixel-agents/layout.json` (atomic write, .tmp + rename).
- [ ] K2 (MVP) Config: `~/.pixel-agents/config.json` (external asset directories etc.).
- [ ] K3 (MVP) Agents: per-workspace state including palette/hueShift/seatId.
- [ ] K4 (Full) Layout file watcher (fs.watch + 2s polling), cross-window sync.
- [ ] K5 (Full) `markOwnWrite()` prevents reading our own writes.
- [ ] K6 (Full) Last-save-wins: skip external update if local has unsaved edits.
- [ ] K7 (Full) Default layout fallback chain: user file → migration → bundled `default-layout.json` → procedural.
- [ ] K8 (Full) Export layout / Import layout (file dialogs).
- [ ] K9 (Full) "Export Layout as Default" command.

## L. Multi-Window / Multi-Client

- [ ] L1 (Full) Multiple TUI clients can connect to one daemon simultaneously.
- [ ] L2 (Full) Layout edits in one client propagate to others.
- [ ] L3 (Full) Each client has independent focus/zoom/selection.
- [ ] L4 (Full) Hooks server is multi-window safe (already in spec).

## M. Sound & Notifications

- [ ] M1 (Full) Ascending two-note chime on agent turn completion / waiting bubble.
- [ ] M2 (Full) Sound toggle in settings; persisted.
- [ ] M3 (Full) Cross-platform audio (Linux/macOS/Windows).

## N. UI Chrome / Settings

- [ ] N1 (MVP) Bottom toolbar equivalent: + Agent, Layout, Settings.
- [ ] N2 (Full) Settings modal: sound, hooks, debug, asset directories, export/import.
- [ ] N3 (Full) Info modal / changelog viewer.
- [ ] N4 (Full) First-run tooltip with dismiss.
- [ ] N5 (Full) Debug overlay: per-agent JSONL diagnostics.
- [ ] N6 (Full) Pixel-art aesthetic preserved (sharp corners, hard shadows, FS Pixel Sans equivalent in terminal).

## O. Distribution & Install

- [ ] O1 (MVP) Single-command install (npm/cargo/brew).
- [ ] O2 (MVP) `pixel-agents` binary launches both daemon and TUI client appropriately.
- [ ] O3 (Full) Daemon survives client disconnect; clients can reattach.
- [ ] O4 (Full) Auto-update channel or clear upgrade path.

## P. Testing

- [ ] P1 (Full) Unit tests on daemon (parity with current `server/__tests__/`).
- [ ] P2 (Full) JSONL parser tests (parity with current parser).
- [ ] P3 (Full) TUI snapshot tests (`insta` or equivalent).
- [ ] P4 (Full) E2E test against a real `claude` session.
- [ ] P5 (Full) Terminal capability fallback tested (Kitty / Sixel / 24-bit / 256 / 16-color).

## Q. Cross-Platform

- [ ] Q1 (MVP) Linux support (terminals: Kitty, WezTerm, foot, alacritty, gnome-terminal, xterm).
- [ ] Q2 (MVP) macOS support (Terminal.app limitations documented; iTerm2/WezTerm/Kitty first-class).
- [ ] Q3 (Full) Windows support (Windows Terminal first-class; conhost.exe limitations documented).
- [ ] Q4 (Full) Tmux / Zellij / Screen detected; behavior documented (some graphics protocols don't pass through).

## R. Performance Targets

- [ ] R1 (MVP) ≤16ms frame budget at 60fps in Kitty graphics tier.
- [ ] R2 (MVP) ≤33ms frame budget at 30fps in fallback tiers.
- [ ] R3 (Full) <100MB RSS for daemon + one client.
- [ ] R4 (Full) <5% CPU idle with 5 agents.

## S. Roadmap / Stretch (do NOT need parity but shouldn't be architecturally blocked)

- [ ] S1 Agent-agnostic adapters (Codex, OpenCode, Gemini, Cursor).
- [ ] S2 Kanban-board wall, drag-to-assign.
- [ ] S3 Token health bars.
- [ ] S4 3D / VR future.

---

**Exit criterion for design loop:** every MVP item is ✓; every Full item is ✓ or ⚠ with explicit, documented acceptance; no items are ✗.
