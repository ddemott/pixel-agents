# Critique Round 2 — TUI Architecture v2

## Summary

- **1 critical issue** newly introduced (Ratatui version mix-up)
- **6 major issues** (1 partial, 5 new)
- **7 minor issues** (mostly section-to-section inconsistencies)
- **Verdict: needs round 3 — surgical scope, ~3.5 hours of edits**

The architecture is conceptually right. v2 cleanly fixes all 5 previous CRITs and 11/12 MAJs. Remaining issues are editorial, not redesign.

---

## Previous Critique Resolution Audit

### Critical (all 5 ✓ fixed)

- **CRIT-1** PTY parser → ✓ — `wezterm-term 0.22+` with `PtyByteTap` for image passthrough; compilable code sample.
- **CRIT-2** A4 restore → ✓ — OS supervisor (systemd/launchd/Scheduled Task) + `claude --resume` per agent. Seven failure paths spec'd.
- **CRIT-3** Game loop → ✓ — Client-side FSM, daemon broadcasts events + worldSeed; worked example included.
- **CRIT-4** MessageSender → ✓ — Phase 0 milestone gate with concrete interfaces, grep output, day estimates.
- **CRIT-5** server.json arbitration → ✓ — Cooperative `daemon.json`/`server.json` with bootId UUID rotation.

### Major (11/12 ✓ fixed; 1 ⚠ with new factual error)

- MAJ-1 wire schemas → ✓ (small NEW-MIN-1: assets.requestBlob stream EOF)
- MAJ-2 binary mux → ✓ (small NEW-MIN-2: no inbound-PTY tag)
- MAJ-3 capability detection → ✓
- MAJ-4 Kitty placeholders → ⚠ NEW-MAJ-2: Ghostty claim factually wrong vs primary sources
- MAJ-5 Sixel fps targets → ✓
- MAJ-6 PTY resize → ✓ (small NEW-MAJ-4: focus arbitration when 2+ clients focused on same agent)
- MAJ-7 audio/notify cascade → ✓
- MAJ-8 cache budget → ⚠ NEW-MAJ-3: §8/§13/§19 numbers disagree
- MAJ-9 paste/mouse per mode → ✓
- MAJ-10 auto-update → ✓
- MAJ-11 writer-tag → ✓
- MAJ-12 F6 half-block ⚠ → ✓

### Minor (all 9 ✓ fixed)

---

## New Issues

### NEW-CRIT-1: Ratatui version pin is wrong for May 2026

**Where:** §6 library table.
**Issue:** Doc pins `ratatui 0.29.x` + `ratatui-crossterm 0.29.x` — but `ratatui-crossterm` only exists as a separate crate in Ratatui 0.30+ (modularization release, 2025). In 0.29 the backend is `ratatui::backend::CrosstermBackend` inside the monolithic crate.
**Fix:** Pin to `ratatui 0.30.x` + `ratatui-crossterm 0.1.x` (the new workspace crates), OR `ratatui 0.29` with feature flag `["crossterm"]` and no separate crate entry. Verify against crates.io. Same audit for `tachyonfx`.

### NEW-MAJ-1: `world.snapshot` delivery contract ambiguous

**Where:** §5 + §10.
**Issue:** `HelloAck` schema has no `world` field; event table says world.snapshot is "helloAck-adjacent" — undefined ordering.
**Fix:** Either inline `world: WorldSnapshot` in `HelloAck`, OR state explicit invariant: "Daemon MUST send exactly one `evt { topic: 'world.snapshot' }` immediately after HelloAck, before any other event."

### NEW-MAJ-2: Ghostty placeholder claim wrong

**Where:** §7 — demotes Ghostty to T1-O citing "subtle row-anchoring quirks."
**Issue:** Primary sources (Hashimoto posts, arewesixelyet.com) state Ghostty supports unicode placeholders, including through tmux — explicitly the opposite.
**Fix:** Elevate Ghostty to T1-K (matches reality); runtime probe is the escape hatch if a real bug exists.

### NEW-MAJ-3: Three different daemon memory budgets across sections

**Where:** §8 (30 MB), §13 (50 MB raw + 10 MB hot), §19 (30 MB). §8 scrollback math wrong (claims 30 MB; actual = 1.25 MB for 5×256KB).
**Fix:** Pick one canonical budget table; reference it from each subsection. Lift §19 as canonical.

### NEW-MAJ-4: Focus arbitration under-specified

**Where:** §9 + §10 `agent.focus`.
**Issue:** Two clients can both claim focus on agent 7 at different sizes. Daemon "stores focusedClient = A" — but protocol is silent on what happens when B requests focus.
**Fix:** Spec last-focus-wins or bounding-rect; spec daemon notifying prior focused client.

### NEW-MAJ-5: `pty.input` over NDJSON vs binary mux out-of-order

**Where:** §10 pty.input control + 0x01 PTY output.
**Issue:** Two channels, separate receive paths. No ordering invariant stated.
**Fix:** Add inbound-PTY binary tag (0x03), OR document "no cross-channel ordering; kernel TTY arbitrates."

### NEW-MAJ-6: Grep counts disagree (77 in v1 critique, 41 in v2)

**Where:** §11.
**Issue:** Numerical discrepancy unreconciled.
**Fix:** Re-run grep; show actual count; explain delta (probably `import vscode` vs other usages).

### NEW-MIN-1: assets.requestBlob stream EOF unspecified

### NEW-MIN-2: See NEW-MAJ-5

### NEW-MIN-3: §16 "per-client globally guarded" phrasing confused

### NEW-MIN-4: §4 supervisor restart-on-clean-exit needs explicit cross-reference

### NEW-MIN-5: kittyImageId / sha1 cache key relationship unstated

### NEW-MIN-6: Homebrew formula §17 promised but not specified — mark roadmap or specify

### NEW-MIN-7: §22 ⚠ items N3, N6 lack body-section discussion

---

## Parity Checklist Final Audit

- All MVP items ✓ (defensibly).
- C1–C11 bulk-ticked — flag for implementation-plan first-PR review.
- F6 ⚠ defensible; N3 / N6 ⚠ lack body discussion (NEW-MIN-7).
- R3 ⚠ defensible but supporting numbers don't reconcile (NEW-MAJ-3).
- No ✗ items.

## Verdict

**Needs round 3 — narrow editorial fixes.** Total author time ~3.5 hours.

Priority order:

1. NEW-CRIT-1 (Ratatui versions) — blocks first `cargo build`
2. NEW-MAJ-1 (world.snapshot ordering)
3. NEW-MAJ-2 (Ghostty factual fix)
4. NEW-MAJ-3 (memory budgets reconciliation)
5. NEW-MAJ-4 + NEW-MAJ-5 (focus + channel ordering)
6. NEW-MAJ-6 (grep reconciliation)
7. NEW-MIN-1..7 polish

After round 3: ready to write the implementation plan.
