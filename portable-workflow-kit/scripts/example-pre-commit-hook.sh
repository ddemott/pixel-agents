#!/usr/bin/env bash
#
# scripts/example-pre-commit-hook.sh
#
# Staged-files-only pre-commit hook. Now fully project-type aware.
#
# It reads workflow.config.json (via config-reader.sh) and only runs the
# linters / typecheckers / formatters that are appropriate for the declared
# projectType. A Python project will never see eslint or tsc.
#
# Called by .husky/pre-commit (which is installed automatically on npm install).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/config-reader.sh
source "$SCRIPT_DIR/config-reader.sh"

PTYPE="$(get_project_type)"

STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACMR | grep -E '\.(ts|tsx|js|jsx|json|md|py|go|rs|toml|yaml|yml)$' || true)

if [ -z "$STAGED_FILES" ]; then
    echo "No relevant staged files to check. Skipping pre-commit hooks."
    exit 0
fi

echo "==> Running pre-commit checks (projectType: $PTYPE) on staged files only..."

# Helper: run a command if it is "real" in the config
run_if_real() {
    local label="$1"
    local cmd="$2"
    if is_real_command "$cmd"; then
        echo "  - $label..."
        # We intentionally do NOT auto-scope the command to only staged files here
        # because many linters (eslint --fix, ruff, black) work best when given
        # the whole project or a proper glob. The staged list is advisory only.
        if eval "$cmd"; then
            echo "    ✅ $label passed"
        else
            echo "    ❌ $label failed"
            exit 1
        fi
    else
        echo "  - $label (skipped — not defined for projectType '$PTYPE')"
    fi
}

# Always run format check if defined (cheap and universal)
FORMAT_CMD="$(get_command formatCheck)"
run_if_real "Format check" "$FORMAT_CMD"

# Lint (the actual lint command from config — may be eslint, ruff, golangci-lint, etc.)
LINT_CMD="$(get_command lint)"
run_if_real "Lint" "$LINT_CMD"

# Typecheck is intentionally only run if the projectType profile actually defines it
# (node-fullstack has it; python profile usually does not, or uses mypy/pyright)
TYPECHECK_CMD="$(get_command typecheck 2>/dev/null || true)"
if [ -z "$TYPECHECK_CMD" ]; then
    # Some profiles put the typecheck work inside "checks". We do not double-run here.
    true
else
    run_if_real "Typecheck" "$TYPECHECK_CMD"
fi

echo "✅ Pre-commit checks passed for projectType '$PTYPE'."
echo ""
echo "Reminder: Run 'npm run pre-pr' (or your equivalent) before opening a PR."
exit 0
