#!/usr/bin/env bash
#
# scripts/config-reader.sh
#
# Tiny, zero-dependency (beyond python3) helper to read values from workflow.config.json.
# Used by prepare-commit.sh, the example hooks, and create-feature-branch.sh so they
# automatically respect the declared projectType and the exact command strings.
#
# Usage (sourcing):
#   source scripts/config-reader.sh
#   CMD=$(get_command "checks")
#   PTYPE=$(get_project_type)
#   SCAN=$(get_command "focusedTestScan")
#
# It prefers `jq` when available (fast + correct). Falls back to a python -c extractor.
# This is what makes the same scripts work for a Python project with ruff/pytest or a
# Node SaaS project with eslint + tsc + vitest without any changes to the scripts themselves.

set -euo pipefail

CONFIG_FILE="${WORKFLOW_CONFIG_FILE:-workflow.config.json}"

# Find the config file walking upward if needed (supports running from subdirs)
find_config() {
    local dir="$PWD"
    while [ "$dir" != "/" ]; do
        if [ -f "$dir/$CONFIG_FILE" ]; then
            echo "$dir/$CONFIG_FILE"
            return 0
        fi
        dir="$(dirname "$dir")"
    done
    # Fallback to the one next to this script (portable kit case)
    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    if [ -f "$script_dir/../$CONFIG_FILE" ]; then
        echo "$script_dir/../$CONFIG_FILE"
        return 0
    fi
    echo ""
}

CONFIG_PATH="$(find_config)"

# Internal: extract a top-level string value using the best available tool
_get_json_string() {
    local key="$1"
    if [ -z "$CONFIG_PATH" ] || [ ! -f "$CONFIG_PATH" ]; then
        echo ""
        return 0
    fi

    # Preferred: jq (if the recipient has it — very common)
    if command -v jq >/dev/null 2>&1; then
        jq -r --arg k "$key" '.[$k] // empty' "$CONFIG_PATH" 2>/dev/null || echo ""
        return 0
    fi

    # Fallback: python (guaranteed on virtually every developer machine in 2026)
    python3 - "$CONFIG_PATH" "$key" 2>/dev/null <<'PY' || echo ""
import json, sys
path, key = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        data = json.load(f)
    val = data.get(key, "")
    if isinstance(val, (dict, list)):
        val = ""
    print(val if val is not None else "")
except Exception:
    print("")
PY
}

# Public API
get_project_type() {
    local t
    t="$(_get_json_string "projectType")"
    if [ -z "$t" ]; then
        echo "node-fullstack"   # safe default that matches historical behavior
    else
        echo "$t"
    fi
}

get_command() {
    local key="$1"
    # First try the active "commands" block (the one the user is supposed to edit)
    local cmd
    if command -v jq >/dev/null 2>&1 && [ -f "$CONFIG_PATH" ]; then
        cmd=$(jq -r --arg k "$key" '.commands[$k] // empty' "$CONFIG_PATH" 2>/dev/null || true)
    else
        # python fallback for the nested case
        cmd=$(python3 - "$CONFIG_PATH" "$key" 2>/dev/null <<'PY' || true
import json, sys
path, key = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        data = json.load(f)
    cmds = data.get("commands", {}) or {}
    val = cmds.get(key, "")
    print(val if val is not None else "")
except Exception:
    print("")
PY
)
    fi

    if [ -n "$cmd" ]; then
        echo "$cmd"
        return 0
    fi

    # Last resort: legacy top-level key (for very old configs)
    _get_json_string "$key"
}

# Convenience: return 0 if the command is "real" (not an echo placeholder)
is_real_command() {
    local cmd="$1"
    if [ -z "$cmd" ]; then return 1; fi
    if echo "$cmd" | grep -qiE '^(echo |true|false|:)'; then
        return 1
    fi
    return 0
}

# Export for sourcing scripts
export CONFIG_PATH
export -f get_project_type
export -f get_command
export -f is_real_command
