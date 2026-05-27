# Portable Development Workflow Kit v1.2.0

This is the minimal set of files needed to adopt the reusable development workflow.

**Project-type aware since v1.2.0.** Set `"projectType": "python"` (or `node-fullstack`, `generic`) in `workflow.config.json` and the automation (`prepare-commit`, Husky hooks, etc.) will only run the tools that actually make sense for that stack. No more forcing `eslint` + `tsc` on a Python codebase.

## Quick Start

1. Copy this entire folder into your project (or use the generator with `--project-type python` etc. for a pre-specialized copy).
2. Follow the instructions in `ADOPTING_THE_WORKFLOW.md`.
3. Edit `workflow.config.json`:
   - Set `projectType`
   - Copy the matching profile into the top-level `commands` block
   - Tweak the exact command strings
4. Run `npm install` (or equivalent) after adding Husky for automatic hook installation.

## Contents

- `workflow.config.json` — The **only** file most adopters need to edit. Contains projectType + per-type command profiles + active commands.
- `BRANCH_CHECKLIST.md` — Generic checklist (commands are resolved from the config at runtime).
- `scripts/` — Automation that reads the config (including the tiny `config-reader.sh` helper).
- `.github/` — PR template, issue templates, and branch protection guidance.

## Reference

Point any team (Node, Python, Go, Rust, mixed) at `ADOPTING_THE_WORKFLOW.md` + this kit and say:

> “Read the adoption guide, copy the kit, set your projectType and commands in the config. You now have the same disciplined process we use — adapted to your tools.”
