#!/usr/bin/env bash
#
# scripts/prepare-commit.sh
#
# Runs as much of the pre-commit / pre-PR checklist as can be automated.
# This is the main automation command for preparing work to be committed.
#
# It is deliberately project-type aware. It reads workflow.config.json
# (via config-reader.sh) so a Python project only runs ruff/pytest/etc.
# and never attempts "npm run lint" or tsc.
#
# Usage:
#   npm run prepare-commit
#   bash scripts/prepare-commit.sh
#
# The exact commands come from the "commands" section of the config.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/config-reader.sh
source "$SCRIPT_DIR/config-reader.sh"

PTYPE="$(get_project_type)"
echo "=========================================="
echo "  PREPARING FOR COMMIT / PR"
echo "  (Automated portion of the workflow)"
echo "  projectType: $PTYPE"
echo "=========================================="
echo ""

FAILED=0

run_or_skip() {
    local label="$1"
    local cmd="$2"
    if is_real_command "$cmd"; then
        echo ">>> $label"
        echo "    Command: $cmd"
        if eval "$cmd"; then
            echo "    ✅ $label passed"
        else
            echo "    ❌ $label failed"
            FAILED=1
        fi
    else
        echo ">>> $label"
        echo "    (skipped — no real command defined for projectType '$PTYPE' in workflow.config.json)"
    fi
    echo ""
}

# 1. Quality checks (format + lint + typecheck equivalent for the type)
CHECKS_CMD="$(get_command checks)"
run_or_skip "1. Running quality checks (format + lint + typecheck)" "$CHECKS_CMD"

# 2. Unit tests
UNIT_CMD="$(get_command unitTests)"
run_or_skip "2. Running unit tests" "$UNIT_CMD"

# 3. Doc drift detector (only if defined)
DRIFT_CMD="$(get_command docDriftCheck)"
run_or_skip "3. Running documentation drift detector" "$DRIFT_CMD"

# 4. Focused test scan (.only / .skip or language equivalent)
SCAN_CMD="$(get_command focusedTestScan)"
echo ">>> 4. Checking for focused / skipped tests (language-appropriate scan)..."
if is_real_command "$SCAN_CMD"; then
    echo "    Command: $SCAN_CMD"
    MATCHES=$(eval "$SCAN_CMD" || true)
    if [ -n "$MATCHES" ]; then
        echo "    ❌ Found focused or skipped tests (review before committing):"
        echo "$MATCHES" | head -20
        FAILED=1
    else
        echo "    ✅ No focused tests found"
    fi
else
    echo "    (skipped — focusedTestScan not defined for this projectType)"
fi
echo ""

# 5. Staged-file heuristics (language-agnostic where possible)
echo ">>> 5. Checking staged files for common issues..."
STAGED=$(git diff --cached --name-only || true)
if [ -n "$STAGED" ]; then
    # Console.log / print debugging statements (covers JS/TS + Python + many others)
    if echo "$STAGED" | xargs grep -l -E "(console\.(log|debug)|^\s*print\(|^\s*debugger;)" 2>/dev/null | head -5; then
        echo "    ⚠️  Found console.log/print/debugger in staged files (review before committing)"
    fi

    # Python-specific: pdb traces left in
    if echo "$STAGED" | grep -qE '\.py$' && echo "$STAGED" | xargs grep -l "pdb.set_trace\|breakpoint()" 2>/dev/null | head -3; then
        echo "    ⚠️  Found pdb breakpoints in staged Python files"
    fi
else
    echo "    (No files staged yet — this check is more useful after 'git add')"
fi
echo ""

echo "=========================================="
if [ "$FAILED" -eq 0 ]; then
    echo "✅ Automated checks completed successfully."
else
    echo "❌ Some automated checks failed. Please fix the issues above."
fi
echo "=========================================="
echo ""

echo "Remaining manual / human steps before using 'commit' with your agent:"
echo ""
echo "  - Review and fix any failures from the checks above"
echo "  - Run relevant E2E / integration tests (command defined in config as 'e2e')"
E2E_CMD="$(get_command e2e)"
if is_real_command "$E2E_CMD"; then
    echo "      $E2E_CMD \"<pattern>\""
else
    echo "      (no E2E command configured for this projectType)"
fi
echo "  - Update documentation (as listed in workflow.config.json under documentation.filesThatMustBeUpdated)"
echo "  - Fill out BRANCH_CHECKLIST.md"
echo "  - Write a good commit message (the commit-code skill will help draft one)"
echo "  - Get explicit approval from the commit-code process before committing"
echo ""
echo "When ready, tell your agent:"
echo "  \"commit\" or \"commit code\""
echo ""

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi
