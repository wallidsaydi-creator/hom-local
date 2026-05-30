#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# release-sanitize.sh — Validate the public release is clean.
#
# This script checks for forbidden content that should never appear in the
# public HOM Local repository. Run it before creating a release.

set -euo pipefail

ERRORS=0

check() {
    local pattern="$1"
    local description="$2"
    local files

    files=$(grep -r -l -E "$pattern" --include='*.rs' --include='*.toml' --include='*.md' --include='*.json' --include='*.yml' --include='*.yaml' --exclude-dir=target --exclude-dir=.git . 2>/dev/null || true)

    if [ -n "$files" ]; then
        echo "FAIL: $description"
        echo "  Found in:"
        echo "$files" | sed 's/^/    /'
        ERRORS=$((ERRORS + 1))
    else
        echo "PASS: $description"
    fi
}

echo "=== HOM Local Release Sanitation ==="
echo ""

# 1. No real brain state databases
check '\.(db|sqlite|sqlite3)$' "No database files"

# 2. No environment files
check '^\.env' "No .env files"

# 3. No API keys or secrets
# Exclude known dummy test keys by filtering out lines with the dummy key
real_api_keys=$(grep -r -E 'sk-[a-zA-Z0-9]{20,}' --include='*.rs' --include='*.toml' --include='*.md' --include='*.json' --include='*.yml' --include='*.yaml' --exclude-dir=target --exclude-dir=.git . 2>/dev/null | grep -v 'sk-abcdefghijklmnopqrstuvwxyz123456' || true)
if [ -n "$real_api_keys" ]; then
    echo "FAIL: No real API keys"
    echo "  Found:"
    echo "$real_api_keys" | head -5 | sed 's/^/    /'
    ERRORS=$((ERRORS + 1))
else
    echo "PASS: No real API keys"
fi

check 'AKIA[A-Z0-9]{16}' "No AWS access key IDs"
check '-----BEGIN (RSA |EC )?PRIVATE KEY-----' "No private keys"

# 4. No Oracle references (the private system)
check '(?:import\.oracle_pack|oracle[_-]alpha[_-]pack)' "No Oracle pack references"
check '"oracle:(?!test)' "No oracle: prefixed keys (except test data)"
check 'oracle-alpha-pack' "No oracle-alpha-pack references"

# 5. No private file paths
check '/Users/walidsaidi/' "No private user paths"

# 6. No backup files
check '\.(bak|bak-[0-9]+)$' "No backup files"

# 7. No .DS_Store
check '\.DS_Store' "No .DS_Store files"

# 8. License check
if grep -q "Apache-2.0" Cargo.toml; then
    echo "PASS: Apache-2.0 license in Cargo.toml"
else
    echo "FAIL: Missing Apache-2.0 license in Cargo.toml"
    ERRORS=$((ERRORS + 1))
fi

if [ -f "LICENSE" ] && grep -q "Apache License" LICENSE; then
    echo "PASS: Apache License in LICENSE file"
else
    echo "FAIL: Missing Apache License in LICENSE file"
    ERRORS=$((ERRORS + 1))
fi

# 9. All crates must have license metadata
for crate_dir in crates/*/; do
    if [ -f "${crate_dir}Cargo.toml" ]; then
        if grep -q 'license' "${crate_dir}Cargo.toml"; then
            echo "PASS: ${crate_dir} has license metadata"
        else
            echo "FAIL: ${crate_dir} missing license metadata"
            ERRORS=$((ERRORS + 1))
        fi
    fi
done

# 10. No "All rights reserved" in source files
check 'All rights reserved' "No 'All rights reserved' in source files"

# 11. No "proprietary" outside docs explaining Oracle is private
proprietary_files=$(grep -r -l -i 'proprietary' --include='*.rs' --include='*.toml' --exclude-dir=target --exclude-dir=.git . 2>/dev/null || true)
if [ -n "$proprietary_files" ]; then
    echo "FAIL: No 'proprietary' in source files"
    echo "  Found in:"
    echo "$proprietary_files" | sed 's/^/    /'
    ERRORS=$((ERRORS + 1))
else
    echo "PASS: No 'proprietary' in source files"
fi

echo ""
echo "=== Results ==="

if [ $ERRORS -eq 0 ]; then
    echo "All checks passed."
    exit 0
else
    echo "$ERRORS check(s) failed."
    exit 1
fi
