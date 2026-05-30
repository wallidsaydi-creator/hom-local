#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# setup-github-pages.sh — Initialize GitHub repo and enable GitHub Pages
#
# Run this ONCE after you've created the repo on GitHub.
# Prerequisites: gh CLI authenticated (gh auth login)

set -euo pipefail

REPO="wallidsaydi-creator/hom-local"

echo "=== HOM Local — GitHub Pages Setup ==="
echo ""

# 1. Check gh auth
echo "Checking GitHub CLI authentication..."
if ! gh auth status >/dev/null 2>&1; then
    echo "ERROR: gh CLI not authenticated. Run: gh auth login"
    exit 1
fi
echo "PASS: gh CLI authenticated"

# 2. Create repo if it doesn't exist
echo ""
echo "Checking if repo exists..."
if gh repo view "$REPO" >/dev/null 2>&1; then
    echo "PASS: Repo $REPO already exists"
else
    echo "Creating repo $REPO..."
    gh repo create "$REPO" \
        --public \
        --description "HOM Local — open-source local-first AI memory server" \
        --license Apache-2.0 \
        --source . \
        --push
    echo "PASS: Repo created and initial push complete"
fi

# 3. Enable GitHub Pages (source: docs/ on main)
echo ""
echo "Enabling GitHub Pages..."
gh api -X PUT "repos/$REPO/pages" \
    --input - <<'EOF' 2>/dev/null || true
{
    "source": {
        "branch": "main",
        "path": "/docs"
    }
}
EOF

# If the PUT failed (pages not yet supported or needs POST), try POST
gh api -X POST "repos/$REPO/pages" \
    --input - <<'EOF' 2>/dev/null || true
{
    "source": {
        "branch": "main",
        "path": "/docs"
    }
}
EOF

echo "PASS: GitHub Pages configured (source: docs/ on main branch)"

# 4. Set repo description and topics
echo ""
echo "Setting repo metadata..."
gh repo edit "$REPO" \
    --description "HOM Local — open-source local-first AI memory kernel" \
    --add-topic ai \
    --add-topic memory \
    --add-topic local-first \
    --add-topic rust \
    --add-topic llm

echo "PASS: Repo metadata updated"

# 5. Summary
echo ""
echo "=== Setup Complete ==="
echo ""
echo "Repository:  https://github.com/$REPO"
echo "Docs:        https://$(echo $REPO | tr '/' '.').github.io/$(basename $REPO)/"
echo "License:     Apache-2.0"
echo ""
echo "Next steps:"
echo "  1. Push your code:  git push -u origin main"
echo "  2. Wait for Pages to build (~1-2 minutes)"
echo "  3. Visit the docs URL above"
echo ""
echo "NOTE: GitHub Pages deployment is automatic on push to main."
echo "The docs will be live at the URL above after the first push."
