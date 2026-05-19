# TODO Index

This repo doesn't keep todos in one place — they live in the document that's authoritative for each kind of work. Use this index as the entry point.

## Active work

- **TUI port build plan** — `docs/tui-implementation-plan.md`
  Phased work for porting Pixel Agents to a standalone TUI. The Progress block at the top tracks which phases/days have shipped. Authoritative for "what to build next."
  - Phase 0 ✅ — MessageSender refactor
  - Phase 1 Day 1 ✅ — daemon scaffold
  - Phase 1 Day 2 ✅ — port `server/` → `daemon/src/hooks/`
  - Phase 1 Day 3-4 ⏭️ — RPC framing (NDJSON + binary mux on UDS)
  - Phase 1 Days 5-16, Phases 2-8 — not started

- **TUI parity checklist** — `docs/tui-parity-checklist.md`
  ~100 feature parity items (MVP vs Full) as Markdown checkboxes. Source of truth for "is the TUI feature-complete?" Items get ticked as features ship.

## Design open questions

- `docs/tui-architecture.md` § "Open Questions"
  Architectural punts the design loop deliberately left for later.

## In-source TODOs

- `grep -rn "TODO" src/ daemon/src --include="*.ts"`
  Notable: `daemon/src/hooks/eventHandler.ts` has a `TODO(Standalone version)` about moving timerManager + types into `daemon/src/` to drop the cross-package import. Cleanup work; not blocking.

## Historical record (do not edit)

These document past decisions and the design loop's progression:

- `docs/critique-r1.md`, `docs/critique-r2.md`
- `docs/changes-r1-to-r2.md`, `docs/changes-r2-to-r3.md`

## Conventions

- Add new items to the document that's authoritative for their kind, not to this file.
- Tick parity checkboxes as features ship.
- Update the implementation plan's Progress block when a phase or day completes.
- Keep this file as an index — no items inline.
