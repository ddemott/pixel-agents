#!/usr/bin/env bash
#
# scripts/setup-hooks.sh
#
# Legacy/manual hook installer.
#
# The recommended and automatic way is now via Husky + the "prepare" script.
# Running `npm install` will automatically set up the hooks in .husky/.

set -euo pipefail

echo "==> Manual hook setup (legacy mode)"
echo ""
echo "The modern recommended way is automatic:"
echo "  Just run 'npm install' — Husky will install the hooks via the 'prepare' script."
echo ""
echo "If you want to force hook installation right now:"
echo "  npm run prepare"
echo ""
echo "The old manual hooks in .git/hooks can still be installed if needed,"
echo "but using the .husky/ directory (managed by Husky) is preferred."