# Changes — v2 → v3 (Critique-R2 Response)

Surgical edits only. The v3 document supersedes v2 in place; no section was rewritten. Every issue raised in `docs/critique-r2.md` is fixed below with a pinpoint reference.

## NEW-CRIT-1 — Ratatui version pin (§1, §6)

- §1 Executive Summary: pin updated from `Ratatui 0.29 + crossterm 0.29` to `Ratatui 0.30 (modular workspace) + ratatui-crossterm 0.1 + crossterm 0.29`.
- §6 Language justifications: rewritten to reference the 0.30 modularization release and `tachyonfx 0.24` compatibility.
- §6 Library choices table: replaced row-by-row. New rows: `ratatui 0.30.x`, `ratatui-core 0.1.x`, `ratatui-crossterm 0.1.x`, `tachyonfx 0.24.x`. Crossterm stays at 0.29. Added a paragraph below the table explaining why v2's pin was wrong and noting the audit date.
- Sources section: added crates.io / docs.rs links for Ratatui 0.30, `ratatui-crossterm`, and `tachyonfx` releases.

Verified May 2026 against [Ratatui v0.30 highlights](https://ratatui.rs/highlights/v030/), [crates.io `ratatui-crossterm`](https://crates.io/crates/ratatui-crossterm), and [junkdog/tachyonfx releases](https://github.com/junkdog/tachyonfx/releases).

## NEW-MAJ-1 — `world.snapshot` delivery contract (§5, §10)

- §10 `HelloAck` TS interface: added `world: WorldSnapshot` field with a docstring stating the initial snapshot ships inline on the response.
- §10 event-topics table: `world.snapshot` row rewritten to clarify that only **subsequent** canonical-state-change snapshots flow as events.
- §5 "What about WorldSnapshot then?" subsection: rewritten to state the initial snapshot is delivered on `HelloAck.world`, not as a separate event.
- §5 daemon-broadcast bullet: replaced "at `hello`-time" with the explicit inline delivery.
- All "helloAck-adjacent" wording removed.

## NEW-MAJ-2 — Ghostty placeholder claim (§7, §20, §21, §22 row, sources)

- §7 fallback ladder ASCII tier diagram: Ghostty moved from T1-O up to T1-K. WezTerm/Konsole 22+/foot 1.21+ remain in T1-O.
- §7 "T1 split" paragraph: rewritten to retract the v2 "subtle row-anchoring quirks" wording and cite three primary sources (Hashimoto X post, Hachyderm thread, ghostty.org/docs/features). Runtime probe retained as an escape hatch.
- §7 test-matrix row for Ghostty bumped from T1-O → T1-K (1.3+).
- §18 terminal compatibility matrix: same bump; added a Tmux 3.4 + Ghostty 1.3+ row (T1-K via Ghostty's unicode-placeholder support in multiplexers).
- §20 cross-platform Linux + macOS bullets updated.
- §21 MIN-1 resolution updated to reflect Ghostty's elevation.
- Sources section: added Hashimoto X post, Hachyderm thread, ghostty.org/docs/features.

## NEW-MAJ-3 — Memory budget reconciliation (§8, §13, §19)

- §19 promoted to **canonical memory-budget table**: an explicit per-bucket table with daemon (~85 MB nominal, 100 MB cap), client (~90 MB nominal, 100 MB cap), combined (~175 MB nominal, 200 MB cap). Each row's source identified.
- §8 cache-budget paragraph: replaced with a short summary that cross-references §19. Explicit correction noted: scrollback rings = 5 × 256 KB = 1.25 MB, not the 30 MB stated in v2 §8.
- §13 eviction subsection: rewritten to cross-reference §19; numeric caps removed in favor of pointing to the single source.
- R3 revision text retained in §19 (from <100 MB → <200 MB).

## NEW-MAJ-4 — Focus arbitration (§9, §10)

- §9 "Resize protocol" subsection retitled to "Focus arbitration & resize protocol" and prepended with a four-step last-focus-wins spec, an `agent.focusLost` event for the prior owner, an `{ ok: true, previousOwner?: clientId }` return for `agent.focus`, and three edge-case bullets (same-client re-focus, focused-client disconnect, simultaneous-request race).
- §10 event-topics table: added `agent.focusLost { id }` row.
- §10 command catalog: `agent.focus` return type updated to `{ previousOwner?: clientId }`.

## NEW-MAJ-5 (and NEW-MIN-2) — `pty.input` vs binary mux ordering (§10)

- §10 channel-multiplex diagram: added a fourth tag `0x03` for inbound binary PTY (large pastes >64 KB), kept `0x01` as outbound-only.
- Added an explicit **Ordering invariant** paragraph: no cross-channel ordering guarantee with outbound; kernel TTY arbitrates; matches conventional terminal behavior.
- `pty.input` row in the command catalog updated to note the 64 KB ceiling and reference 0x03 for larger inputs.

## NEW-MAJ-6 — Grep counts reconciliation (§1, §11)

- §11 grep block now shows the verified 2026-05-19 counts (19 + 3 + 15 + 4 = 41 lines) with an explanatory paragraph explaining the v1 "77+" vs v2/v3 "41" delta as a counting-methodology difference (occurrences-per-identifier vs lines-with-match). Both are correct under their definitions; the Phase 0 gate condition (zero) is unambiguous either way.
- §1 callout bullet replaced "Grep proves … 41 references" with "Verified `grep -c "vscode" …` on 2026-05-19 returns 41 lines (see §11 for reconciliation)" — annotated NEW-MAJ-6.

## NEW-MIN-1 — `assets.requestBlob` stream EOF (§10)

- §10: added an "Asset blob framing (0x02) — chunking & EOF" subsection. Per-frame cap = 1 MB; multi-frame splits share `assetId`; high bit of `tier` byte set on final frame; receiver concatenates in arrival order; no interleaving allowed; hard maximum 16 MB per asset. `assets.requestBlob` command-catalog row updated to point at this subsection.

## NEW-MIN-3 — §16 phrasing ("per-client globally guarded")

- §16 conflict-resolution paragraph: appended an explicit "Suppression policy" sentence as specified in the critique. v2 wording retained but framed as the conflated phrasing being clarified.

## NEW-MIN-4 — §4 supervisor restart-on-clean-exit

- §4 after the three supervisor configurations: added a single paragraph stating all three configurations agree (`Restart=on-failure` + `SuccessExitStatus=0` on systemd; `KeepAlive.SuccessfulExit=false` + `Crashed=true` on launchd; `RestartOnFailure` on Scheduled Task) and cross-referencing the lifecycle table row.

## NEW-MIN-5 — kittyImageId / sha1 cache key relationship (§13)

- §13 T1-K/T1-O bullet: added an inline note explaining `kittyImageId` is lazily allocated on first cache miss for the sha1 key, then memoized; multiple spawns with the same sha1 share the same `kittyImageId`.

## NEW-MIN-6 — Homebrew formula (§17, §22 O1)

- §17 alternative-installs list: Homebrew row marked "post-MVP roadmap; formula to be published after v1.0 ships."
- §22 O1 status: annotated "Homebrew is post-MVP" inline so the parity checklist doesn't pretend MVP includes brew.

## NEW-MIN-7 — §22 N3 / N6 body discussion (§6, §22)

- §6: new subsection "Changelog GIFs (N3) and body font (N6) — what the user actually sees" with per-tier behavior described for both items.
- §22 N3 and N6 rows: updated to point to the new subsection and summarize the limitation per tier.

## Document-meta updates

- Document header: bumped from v2 → v3, references `critique-r2.md` and this `changes-r2-to-r3.md`.
- §24 Files Touched: added `critique-r2.md` and `changes-r2-to-r3.md` to the list.
- Trailing word-count statement updated from ~11,400 to ~13,800 (the v2→v3 edits add roughly 2,400 words, all in the targeted fixes above).
- Trailing "Items couldn't be fully resolved" updated to include the R2-cycle items.

## Counts

- Total Edit-tool edits applied: **24**.
- Word count post-edit (excluding fenced code blocks): **~13,800** (v2 was ~11,400).
- Critique items addressed: **14 of 14** from R2 (NEW-CRIT-1; NEW-MAJ-1..6; NEW-MIN-1, 3, 4, 5, 6, 7; NEW-MIN-2 is the same fix as NEW-MAJ-5).
- Sections untouched: every section's structure preserved; only the targeted lines/paragraphs were modified.

## Verification

```
$ grep -c "vscode" src/{fileWatcher,agentManager,transcriptParser,timerManager}.ts
src/fileWatcher.ts:19
src/agentManager.ts:15
src/transcriptParser.ts:3
src/timerManager.ts:4
```

Total: 41 lines containing `vscode`, matching v2 and v3's §11 claim.

Ratatui / Ghostty / tachyonfx versions audited against crates.io and primary sources on 2026-05-19; citations added to the Sources section.
