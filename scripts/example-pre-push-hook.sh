#!/usr/bin/env bash
#
# scripts/example-pre-push-hook.sh
#
# Stronger pre-push hook. Project-type aware.
#
# Runs the full "checks" + "unitTests" commands from workflow.config.json.
# A Python project will run whatever its "checks" and "unitTests" are
# (e.g. ruff + black + pytest). Never hardcodes npm or tsc.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/config-reader.sh
source "$SCRIPT_DIR/config-reader.sh"

PTYPE="$(get_project_type)"

echo "==> Running pre-push checks (projectType: $PTYPE)..."

CHECKS_CMD="$(get_command checks)"
if is_real_command "$CHECKS_CMD"; then
    echo "  - Running quality checks..."
    if eval "$CHECKS_CMD"; then
        echo "    ✅ Quality checks passed"
    else
        echo "    ❌ Quality checks failed. Fix before pushing."
        exit 1
    fi
else
    echo "  - Quality checks (skipped — not defined for this projectType)"
fi

UNIT_CMD="$(get_command unitTests)"
if is_real_command "$UNIT_CMD"; then
    echo "  - Running unit tests..."
    if eval "$UNIT_CMD"; then
        echo "    ✅ Unit tests passed"
    else
        echo "    ❌ Some unit tests are failing. Fix before pushing."
        exit 1
    fi
else
    echo "  - Unit tests (skipped — not defined for this projectType)"
fi

echo "✅ Pre-push checks passed for projectType '$PTYPE'."
echo ""
E2E_CMD="$(get_command e2e)"
if is_real_command "$E2E_CMD"; then
    echo "Reminder: Consider running relevant E2E/integration tests before opening a PR:"
    echo "  $E2E_CMD \"<your-pattern>\""
fi
echo ""
exit 0
