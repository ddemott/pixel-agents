#!/usr/bin/env bash
#
# scripts/create-feature-branch.sh
#
# Creates a properly named feature branch from the latest main,
# following the project's Development Workflow.
#
# Usage:
#   ./scripts/create-feature-branch.sh feat/my-cool-thing
#   ./scripts/create-feature-branch.sh fix/some-bug
#
# It will:
#   1. Checkout main and pull latest
#   2. Create the new branch
#   3. Print a small checklist of things you should consider doing next

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "Usage: $0 <branch-name>"
  echo "Example: $0 feat/e2e-coverage-gaps"
  echo "         $0 fix/consent-optout-shape"
  exit 1
fi

BRANCH_NAME="$1"

# Basic validation of branch name
if [[ ! "$BRANCH_NAME" =~ ^(feat|fix|test|refactor|docs|chore)/ ]]; then
  echo "Error: Branch name should start with one of: feat/, fix/, test/, refactor/, docs/, chore/"
  echo "You provided: $BRANCH_NAME"
  exit 1
fi

echo "==> Fetching latest main..."
git fetch origin main

echo "==> Checking out main and pulling..."
git checkout main
git pull origin main

echo "==> Creating branch: $BRANCH_NAME"
git checkout -b "$BRANCH_NAME"

# Copy the standard branch checklist into the new branch for local tracking
if [ -f "docs/BRANCH_CHECKLIST.md" ]; then
  cp docs/BRANCH_CHECKLIST.md BRANCH_CHECKLIST.md
  echo "==> Copied BRANCH_CHECKLIST.md into the branch root for tracking"
fi

echo ""
echo "✅ Branch created: $(git branch --show-current)"
echo ""

echo "==> Running initial automated quality gates (this may take a minute)..."
echo ""

# Run automated checks
npm run verify:claude-md 2>&1 | tail -5 || true
echo ""

npm run build 2>&1 | tail -3 || true
echo ""

echo "Initial automated checks completed."
echo ""

echo "Recommended next steps (per docs/DEVELOPMENT_WORKFLOW.md):"
echo "  1. Review any issues from the checks above."
echo ""
echo "  2. If this is non-trivial work, create a GitHub Issue using one of the templates:"
echo "     - Feature: .github/ISSUE_TEMPLATE/feature.md"
echo "     - Bug:     .github/ISSUE_TEMPLATE/bug.md"
echo ""
echo "  3. Edit BRANCH_CHECKLIST.md in this branch root to track your progress."
echo ""
echo "  4. When you're ready to commit, run:"
echo "     npm run prepare-commit"
echo "     (This runs the maximum number of automated checks possible.)"
echo ""
echo "  5. Then use the commit-code process with your agent ('commit' or 'commit code')."
echo ""
echo "Happy coding!"