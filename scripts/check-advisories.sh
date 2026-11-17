#!/usr/bin/env bash
# scripts/check-advisories.sh — deterministic dependency advisory and license gate.
#
# Proves three things, in order:
#   1. `cargo audit` and `cargo deny check` are both green on this tree.
#   2. The suppression list has exactly one meaning in three places: the
#      normative `deny.toml`, the `cargo audit` mirror `.cargo/audit.toml`,
#      and the human mirror in `SECURITY.md` § Advisory Exceptions.
#   3. The gate still fails on an unsuppressed advisory (`--self-test`).
#
# Policy: plans/adrs/ADR-0003-advisory-remediation-and-suppression-policy.md.
#
# This is intentionally *not* part of `scripts/check.sh`: that run must stay
# free of network access and of tool-installation requirements. CI calls this
# script from its own job (`.github/workflows/ci.yml`, `advisory-gate`).
#
# Usage:
#   scripts/check-advisories.sh              # full gate
#   scripts/check-advisories.sh --self-test  # also prove an unsuppressed ID fails
#   scripts/check-advisories.sh --help
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

DENY_CONFIG="deny.toml"
AUDIT_CONFIG=".cargo/audit.toml"
SECURITY_DOC="SECURITY.md"
SELF_TEST=0

usage() {
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
    case "$arg" in
        --self-test) SELF_TEST=1 ;;
        -h|--help) usage; exit 0 ;;
        *)
            echo "FAIL: unknown argument: $arg" >&2
            echo "Try: scripts/check-advisories.sh --help" >&2
            exit 2
            ;;
    esac
done

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

# --- 1. Tool availability ---------------------------------------------------
# A missing tool is an environment failure, never a silent pass.
if ! command -v cargo-audit >/dev/null 2>&1; then
    echo "FAIL: cargo-audit is not installed." >&2
    echo "  Install: cargo install cargo-audit --locked --version 0.23.0" >&2
    exit 1
fi

if ! command -v cargo-deny >/dev/null 2>&1; then
    echo "FAIL: cargo-deny is not installed." >&2
    echo "  Install: cargo install cargo-deny --locked" >&2
    exit 1
fi

for config in "$DENY_CONFIG" "$AUDIT_CONFIG" "$SECURITY_DOC"; do
    [ -f "$config" ] || fail "required file is missing: $config"
done

# --- 2. Advisory and license tools -----------------------------------------
echo "=== cargo audit ==="
if ! cargo audit; then
    fail "cargo audit reported an unsuppressed finding (see output above)"
fi

echo "=== cargo deny check ==="
if ! cargo deny check; then
    fail "cargo deny check failed (see output above)"
fi

# --- 3. Mirror consistency --------------------------------------------------
# Extract the quoted advisory IDs from a TOML file, ignoring comment lines so
# that a rationale block naming an ID is not mistaken for a live suppression.
toml_ignore_ids() {
    local file="$1"
    sed 's/#.*$//' "$file" \
        | grep -oE '"RUSTSEC-[0-9]{4}-[0-9]{4}"' \
        | tr -d '"' \
        | sort -u
}

# The SECURITY.md marker is the machine-readable half of the human mirror.
marker_ids() {
    local file="$1"
    sed -n 's/.*<!--[[:space:]]*advisory-ignore-ids:\(.*\)-->.*/\1/p' "$file" \
        | tr ' ' '\n' \
        | grep -E '^RUSTSEC-[0-9]{4}-[0-9]{4}$' \
        | sort -u
}

DENY_IDS="$(toml_ignore_ids "$DENY_CONFIG")"
AUDIT_IDS="$(toml_ignore_ids "$AUDIT_CONFIG")"
DOC_IDS="$(marker_ids "$SECURITY_DOC")"

echo "=== Suppression mirror ==="
echo "deny.toml:        ${DENY_IDS:-<none>}"
echo ".cargo/audit.toml: ${AUDIT_IDS:-<none>}"
echo "SECURITY.md:      ${DOC_IDS:-<none>}"

if [ "$DENY_IDS" != "$AUDIT_IDS" ]; then
    fail "$DENY_CONFIG and $AUDIT_CONFIG suppress different advisory sets"
fi

if [ "$DENY_IDS" != "$DOC_IDS" ]; then
    fail "$DENY_CONFIG and $SECURITY_DOC suppress different advisory sets (check the advisory-ignore-ids marker)"
fi

if [ -z "$DENY_IDS" ]; then
    echo "No active advisory suppressions."
else
    # Every entry must carry the six-field block from ADR-0003. Counting the
    # rationale blocks is enough to catch a copy-pasted or truncated entry.
    for id in $DENY_IDS; do
        count="$(grep -c "ID:[[:space:]]*$id" "$DENY_CONFIG" || true)"
        if [ "$count" -lt 1 ]; then
            fail "$id has no 'ID: $id' rationale block in $DENY_CONFIG"
        fi
        # The rationale block is the run of comment lines immediately above the
        # entry, so a field cannot be borrowed from a neighbouring block.
        if ! awk -v id="$id" '
            function check_block(   f, missing) {
                missing = ""
                for (f = 1; f <= nfields; f++) {
                    if (block !~ fields[f]) { missing = missing " " fields[f] }
                }
                if (missing != "") {
                    printf "missing:%s\n", missing > "/dev/stderr"
                    bad = 1
                }
            }
            BEGIN {
                nfields = split("Scope:|Why:|Added:|Review by:|Remove when:", fields, "|")
                bad = 0
                nblock = 0
            }
            /^[[:space:]]*#/ {
                block = (nblock == 0) ? $0 : block "\n" $0
                nblock++
                next
            }
            {
                if ($0 ~ "\"" id "\",") { check_block() }
                block = ""
                nblock = 0
            }
            END { exit(bad ? 1 : 0) }
        ' "$DENY_CONFIG"; then
            fail "$id has an incomplete rationale block in $DENY_CONFIG"
        fi
    done

    # A suppression entry must be a bare advisory ID: no crate names, versions,
    # or wildcards are permitted by ADR-0003.
    IGNORE_BODY="$(awk '
        /^ignore[[:space:]]*=[[:space:]]*\[/ { in_array = 1 }
        in_array { print }
        in_array && /\]/ { in_array = 0 }
    ' "$DENY_CONFIG" | sed 's/#.*$//')"

    BAD_ENTRY="$(printf '%s\n' "$IGNORE_BODY" \
        | grep -oE '"[^"]*"' \
        | grep -vE '^"RUSTSEC-[0-9]{4}-[0-9]{4}"$' || true)"
    if [ -n "$BAD_ENTRY" ]; then
        fail "$DENY_CONFIG suppresses a non-ID entry: $BAD_ENTRY"
    fi
fi

# --- 4. Negative proof ------------------------------------------------------
if [ "$SELF_TEST" -eq 1 ]; then
    echo "=== Self-test: an unsuppressed advisory must fail ==="
    FIRST_ID="$(printf '%s\n' "$DENY_IDS" | head -n 1)"
    if [ -z "$FIRST_ID" ]; then
        fail "--self-test needs at least one suppression to remove"
    fi

    TMP_DIR="$(mktemp -d)"
    trap 'rm -rf "$TMP_DIR"' EXIT

    awk -v id="$FIRST_ID" '
        $0 == "    \"" id "\"," { next }
        { print }
    ' "$DENY_CONFIG" > "$TMP_DIR/deny.toml"

    if grep -q "\"$FIRST_ID\"," "$TMP_DIR/deny.toml"; then
        fail "could not construct the unsuppressed test config for $FIRST_ID"
    fi

    echo "Removed $FIRST_ID from a temporary copy; expecting a non-zero exit."
    # `--config` is a global flag in cargo-deny: it must precede the subcommand.
    if SELF_TEST_OUTPUT="$(cargo deny --config "$TMP_DIR/deny.toml" check advisories 2>&1)"; then
        fail "self-test failed: the gate passed with $FIRST_ID unsuppressed"
    fi
    # Fail for the right reason: the output must name the advisory we removed.
    # Without this, a bad invocation (wrong flag position, broken config path)
    # would look like a successful negative proof.
    if ! printf '%s\n' "$SELF_TEST_OUTPUT" | grep -q "$FIRST_ID"; then
        printf '%s\n' "$SELF_TEST_OUTPUT" >&2
        fail "self-test failed for the wrong reason: output does not name $FIRST_ID"
    fi
    echo "Self-test passed: the gate still catches an unsuppressed advisory."
fi

echo "=== Advisory and license gate: PASS ==="
