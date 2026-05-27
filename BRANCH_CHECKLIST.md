# Branch Checklist

**Pixel Agents note**: This checklist is used together with the project's existing disciplined process in `CLAUDE.md` (slash commands `/start-feature`, `/commit-code`, mandatory 5W comments in tests, and strict documentation updates).

Use this checklist when starting and finishing work on a feature branch.

**This checklist is generic.** The actual commands you run come from `workflow.config.json`
(under `commands` and `documentation`). A Python project will run `ruff` + `pytest`
instead of `eslint` + `vitest`. Always consult your project's `workflow.config.json`.

## When Starting a Branch

- [ ] Branch created using `npm run create-branch <type>/<name>` (or the script directly)
- [ ] Branch name follows convention (`feat/`, `fix/`, `test/`, `refactor/`, `docs/`, `chore/`)
- [ ] GitHub Issue created (or existing issue linked) if the work is non-trivial
- [ ] Initial quality gates run (use the commands from your `workflow.config.json`):
  - [ ] doc-drift detector (usually `commands.docDriftCheck`)
  - [ ] build (if your project has one)
  - [ ] unit tests (`commands.unitTests`)
- [ ] Local tracking started (this checklist, issue, or `docs/TODO.md` entry)

## While Developing

- [ ] Code follows existing patterns in the module
- [ ] `commands.checks` (or your project's equivalent) passes frequently
- [ ] Relevant unit tests written/updated with 5W comments
- [ ] Relevant E2E / integration tests written/updated (use your `commands.e2e` + `--grep` style filtering)
- [ ] Documentation updated as changes are made (not saved for the end)
- [ ] No focused/skipped tests introduced (the scan respects your projectType)

## Before Considering the Work "Done"

- [ ] Lint + format clean (run whatever `commands.lint` + `commands.formatCheck` resolve to for your projectType)
- [ ] Type checking clean (only if your projectType defines a typechecker — e.g. mypy for Python, tsc for TS)
- [ ] Full build succeeds (if your project has a build step)
- [ ] Relevant unit tests pass
- [ ] Relevant E2E/integration tests pass (targeted)
- [ ] All new or modified tests have proper 5W diagnostic comments
- [ ] Documentation updated (the files listed in `documentation.filesThatMustBeUpdated` in your config)
- [ ] Doc drift detector passes (your `commands.docDriftCheck`)
- [ ] No test focused/skips left behind (the scan is language-aware via `commands.focusedTestScan`)
- [ ] No secrets, large binaries, or unintended files staged
- [ ] Pre-PR checklist completed (your project's `pre-pr` script or the equivalent of `npm run pre-pr`)

## Before Committing / Pushing

- [ ] Use the `commit-code` process (tell your agent "commit" or "commit code")
- [ ] Commit message follows Conventional Commits style
- [ ] Pushed to the feature branch
- [ ] Pull Request opened with the proper template filled out

---

**Remember**: This checklist exists to protect future-you (and the project). Skipping steps creates technical debt and painful debugging later.
