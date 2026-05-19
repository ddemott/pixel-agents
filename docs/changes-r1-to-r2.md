# Changes from v1 → v2 (Critique-R1 Response)

This document summarizes the deltas between `tui-architecture.md` v1 and v2, indexed by critique item.

## Critical issues

### CRIT-1 — PTY parser strategy

**Changed:** v1 chose `alacritty_terminal 0.25` as the headless emulator with vague claims about image passthrough. v2 switches to **`wezterm-term 0.22+`** because:

- It is explicitly designed to be embedded (no GUI/PTY deps; consumes bytes via `advance_bytes()`).
- It has first-class Sixel and iTerm2 inline image cells.
- It is the parser used by WezTerm itself, continuously battle-tested.

For Kitty graphics protocol (which is pixel-buffer, not grid), v2 specifies a **raw-byte tap upstream of the grid parser** with a `PtyByteTap` Rust trait interface, plus a compilable code sample. `alacritty_terminal` rejected for embedding role (image escapes silently dropped, API annotated unstable). `vt100-rust` rejected (slow maintenance, no image support).

**Sections affected:** §1, §6, §9.

### CRIT-2 — A4 "Restore agents on relaunch" demoted from ⚠ to ✓

**Changed:** v1 admitted ⚠ on an MVP item. v2 specifies **both** approaches:

- **OS supervisor**: systemd-user / launchd / Scheduled Task. Unit files provided verbatim in §4.
- **`claude --resume` revival**: On daemon boot, `restoreAgentsOnBoot()` reads `agents.json`, runs `claude --resume <sessionId>` per entry, with explicit failure paths (JSONL missing, stale >30 days, binary missing, session expired, version mismatch, hung).

**Sections affected:** §4, §16.

### CRIT-3 — Game-loop authority moved to clients

**Changed:** v1 made the daemon authoritative with 30 Hz snapshots and client interpolation. This broke F6 pixel-perfect animation and ignored that the engine code is already pure. v2 moves **FSM/animation authority to clients**:

- Daemon broadcasts **events + canonical world model + `worldSeed`**.
- Each client runs its own `OfficeState` FSM at 60 Hz, seeded by `worldSeed XOR agentId`.
- Worked example (one agent walks-to-seat, types Write, returns to wander) shows exactly 3 events for a 20-second sequence.
- Synchronization-critical effects (matrix spawn/despawn) carry absolute `t0` in event payload.

**Sections affected:** §5, §11, §22 (G-section verification).

### CRIT-4 — Phase 0 MessageSender interface

**Changed:** v1 claimed "port verbatim" but grep showed 41 `vscode` references across the four reused files. v2:

- Defines `AgentEventSink` interface with `broadcast(event)` and `emitTo(agentId, event)`.
- Lists per-file refactor scope: transcriptParser (1d), timerManager (0.5d), fileWatcher (2-3d with `TerminalRegistry`), agentManager (3-4d with `AgentRuntime` + `AgentStateStore`).
- Provides verbatim grep output of the 41 offending lines.
- Phase 0 milestone gate: `grep "vscode" src/{four-files}.ts` returns zero; all tests green; behavior observably identical.
- Only after Phase 0 ships does the daemon port begin.

**Sections affected:** §1 (headline decision 3), §11 (full rewrite of the §11 reused-vs-rewritten table with post-Phase 0 status).

### CRIT-5 — server.json multi-process arbitration

**Changed:** v1 unilaterally demanded "exit if another daemon detected", which conflicted with the live VS Code extension server. v2:

- **Cooperative-with-extension resolution rule**: launcher reads both `daemon.json` and `server.json`. Whichever boots first wins; the other adopts.
- `bootId` UUIDv4 rotation: per process start; clients pin to `bootId` returned in `hello`; mismatched `bootId` = dead connection.
- Cold-start retry: 3 s (was 2 s); two probes at 250 ms and 1 s; one-shot daemon respawn if both fail.
- Corruption recovery: unreadable `daemon.json` → log + unlink + proceed-as-absent.
- Hook script discovery: `PIXEL_AGENTS_HOOK_URL` → `daemon.json` → `server.json` chain.

**Sections affected:** §4, §15.

## Major issues

### MAJ-1 — Wire protocol schemas

**Changed:** v1 sketched event topics in prose. v2 provides full TypeScript schemas for `WireMessage`, `WorldSnapshot`, `AgentSnapshot`, `ClientCapabilities`, and the entire event topic table. Map → tuple-array, Set → T[] serialization explicitly specified.

**Section affected:** §10.

### MAJ-2 — Binary multiplexed PTY channel

**Changed:** v1 sent PTY bytes as base64 strings in 4 MB NDJSON lines. v2 specifies a **length-prefixed binary multiplex**: byte-tag-0x01 + streamId:u32 + len:u32 + bytes[len]. NDJSON max line drops from 4 MB → 256 KB (control only). Asset blobs use 0x02 type tag.

**Section affected:** §10.

### MAJ-3 — Capability detection pre-app input queue

**Changed:** v1's probe ladder raced against user keystrokes. v2 specifies a **pre-app input drain thread** (vte 0.13 parser) that buffers non-response bytes into a `VecDeque<KeyEvent>`. Main thread drains this queue before reading new input. `PIXEL_AGENTS_TIER` env override added.

**Section affected:** §7.

### MAJ-4 — Kitty placeholders only on Kitty

**Changed:** v1's T1 tier lumped Kitty/Ghostty/WezTerm/Konsole as supporting unicode placeholders. Reality (May 2026): only Kitty does so robustly. v2 splits T1 into:

- **T1-K** (Kitty only): unicode placeholders with `U=1`.
- **T1-O** (Ghostty, WezTerm, Konsole 22+, foot 1.21+): non-virtual placement `a=T` without `U=1`. Images don't survive scrollback (documented caveat).

Runtime probe verifies placeholder support: emit placeholder cell, force-scroll 1 row, read back with `\x1b[6n`, check anchor.

**Sections affected:** §7, §8.

### MAJ-5 — Per-terminal Sixel fps targets

**Changed:** v1 promised 30 fps Sixel everywhere. v2 specifies per-terminal targets:

- T3-foot/WezTerm/mlterm: 30 fps.
- T3-xterm: **15 fps** (xterm Sixel parser is slow).
- T3-Windows Terminal: 20 fps.

R2 status revised to ⚠ for T3-xterm only.

**Sections affected:** §8, §19, §22.

### MAJ-6 — PTY resize follows focused client

**Changed:** v1's "smaller dimension wins" was wrong (clobbers active user view). v2:

- PTY size follows the focused client; focus changes debounced 250 ms.
- Non-focused clients render scaled-down preview via grid resampling.
- Focus storm: only last winner triggers `pty.resize`.

**Section affected:** §9.

### MAJ-7 — Audio cascade + cross-window notifications

**Changed:** v1 missed PipeWire and lacked cross-window notify. v2:

- **Linux**: pw-play → paplay → aplay → bell.
- **macOS**: afplay → `osascript -e 'beep 1'`.
- **Windows**: PowerShell SoundPlayer → cmd BEL.
- **Desktop notifications** (separate from audio): `notify-send` (Linux), `terminal-notifier` or `osascript -e 'display notification'` (macOS), PowerShell toast or BurntToast (Windows). Rate-limited 1/agent/5s. Fire on `agentStatus: waiting|permission` and only when daemon's foreground client isn't focused on that agent.

**Section affected:** §14.

### MAJ-8 — Asset cache budget reconciled

**Changed:** v1 reserved 200 MB / client + 50 MB / daemon — incoherent with R3 <100 MB. v2:

- **Daemon**: raw RGBA only + tier-key bookkeeping; 30 MB hot tier-blob LRU cache.
- **Client**: 50 MB LRU on tier representations for _current scene only_.
- Hue-shifted character variants: **lazy on character spawn** (was pre-cached 8 positions × 6 palettes in v1, ~40 MB wasted).
- R3 revised target: <200 MB combined (acknowledged as honest).

**Sections affected:** §8, §13, §19, §22.

### MAJ-9 — Bracketed paste & mouse capture per focus mode

**Changed:** v1 didn't specify. v2 specifies per-mode:

| Mode     | Bracketed paste       | Mouse                      |
| -------- | --------------------- | -------------------------- |
| Office   | Disabled              | Client captures (SGR)      |
| PtyAgent | Passed through to PTY | PTY captures unless we own |
| Editor   | Disabled              | Client captures            |
| Modal    | Disabled              | Modal consumes             |

**Section affected:** §6.

### MAJ-10 — Auto-update opt-in, no in-place upgrade

**Changed:** v1 had `autoUpdate: 'check' | 'apply' | 'off'` with apply doing in-place `npm install -g`. v2:

- **Default: off.**
- `autoUpdate: "check"` only (banner with release notes link).
- **There is no `"apply"` mode.** User runs `npm install -g pixel-agents@latest` themselves.
- SHA verified against **npm-resolved** package only (`bin/manifest.json` in the npm tarball).

**Section affected:** §17.

### MAJ-11 — Layout writer-tag

**Changed:** v1 used `markOwnWrite()` with a wall-clock 750 ms proximity check, which is racy (filesystem clock drift, hibernation). v2 embeds `_writer: { processId, bootId }` at the end of `layout.json` on every write. File watcher parses, compares `_writer.bootId` to daemon's own `bootId`: match → silent; mismatch → external apply + broadcast.

**Section affected:** §16.

### MAJ-12 — F6 demoted to ⚠ on half-block tiers

**Changed:** v1 claimed F6 ✓ everywhere. v2 acknowledges horizontal pixel doubling on T4/T5/T6 (half-block cells are ~16h × ~8w, so 1 cell-column = 1 sprite pixel column but 1 cell-row = 2 sprite rows). F6 status:

- T1-K, T1-O, T2, T3: ✓ pixel-perfect.
- T4/T5/T6: ⚠ accepted limitation; sprites appear 2:1 horizontally stretched.

MVP requirement satisfied because at least one tier (T1-K) is pixel-perfect.

**Sections affected:** §8, §22.

## Minor issues

- **MIN-1** OQ-2 (Kitty placeholders in WezTerm) promoted from open to **blocking and addressed** via T1-K/T1-O split (§7).
- **MIN-2** tmux passthrough probe nesting: probe per-layer via counted `\ePtmux;` prefixes; warn on nested without all layers configured (§15).
- **MIN-3** Hook auth token rotation: generated at install, rotated only via explicit `--rotate-token`; in-flight events have ≤500 ms re-auth window (§15).
- **MIN-4** Schema versioning: `schemaVersion: 1` on `WorldSnapshot`; `protoVersion` in `hello`; mismatched majors refused (§10).
- **MIN-5** Logging: `~/.pixel-agents/logs/daemon-YYYY-MM-DD.log`, NDJSON, daily rotation, gz after 7d, delete after 30d (§16).
- **MIN-6** Tab vs claude tab-complete: Tab sends literal Tab to PTY in PtyAgent mode; pane-switch rebound to Ctrl+Alt+O (§6).
- **MIN-7** Keymap customization: `~/.pixel-agents/keymap.toml` (§6).
- **MIN-8** Citation hygiene: dasroot.net dropped; replaced with primary sources only (§ Sources).
- **MIN-9** Naming: `pixel-agents` = user command + npm pkg; `pixel-agents-tui` = Rust client binary; `pixel-agents-daemon` is only a subcommand (§17).

## Other significant changes (not directly tied to a critique item)

- The `§21 Parity Checklist Status` is now §22, fully rewritten — every MVP is ✓; every Full item is ✓ or ⚠ with explicit acceptance.
- Old §22 Open Questions reduced from 10 to 4; the rest resolved inline.
- Added explicit "Failure paths" tables for both `--resume` revival (§16) and PTY failure modes (§9).
- Added §21 Minor-Issue Resolutions to give MIN items a single home.
- v1's "Note on delivery: I am running in read-only planning mode" preamble removed.
- §1 reorganized to lead with the four headline decisions that resolve CRIT-1..CRIT-5.

## What did _not_ change

- §12 Layout Editor (preserved nearly verbatim — the critique called it strong).
- §17 Distribution npm packaging structure (only adds opt-in supervisor install and tightens auto-update).
- §18 Testing approach (Vitest + cargo test + insta snapshots + Docker matrix).
- §20 Cross-Platform notes (only updated to reflect T1-K/T1-O split and Sixel per-terminal fps).
