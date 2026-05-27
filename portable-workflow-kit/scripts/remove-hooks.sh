#!/usr/bin/env bash
#
# scripts/remove-hooks.sh
#
# Removes manually installed Git hooks.
#
# Note: With Husky, hooks are managed in .husky/ and installed automatically.
# This script mainly cleans up legacy .git/hooks entries.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOKS_DIR="$REPO_ROOT/.git/hooks"

echo "==> Removing legacy local Git hooks (if any)..."

if [ -f "$HOOKS_DIR/pre-commit" ]; then
  rm "$HOOKS_DIR/pre-commit"
  echo "   ✓ Removed legacy pre-commit hook"
fi

if [ -f "$HOOKS_DIR/pre-push" ]; then
  rm "$HOOKS_DIR/pre-push"
  echo "   ✓ Removed legacy pre-push hook"
fi

echo ""
echo "Legacy hooks cleaned."
echo "Husky-managed hooks (in .husky/) are not affected by this script."
echo "To fully disable hooks, you can also run: npm pkg delete scripts.prepare"