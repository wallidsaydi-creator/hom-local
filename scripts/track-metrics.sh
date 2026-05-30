#!/usr/bin/env bash
# Track HOM Local metrics from GitHub and crates.io
# Usage: ./scripts/track-metrics.sh

set -euo pipefail

REPO="wallidsaydi-creator/hom-local"
METRICS_DIR="docs/metrics"
mkdir -p "$METRICS_DIR"

DATE=$(date -u '+%Y-%m-%d')
CSV="$METRICS_DIR/metrics.csv"

# Create CSV header if it doesn't exist
if [ ! -f "$CSV" ]; then
  echo "date,stars,forks,open_issues,hom_brain_downloads,hom_shared_downloads,hom_ingress_downloads,total_downloads" > "$CSV"
fi

echo "Fetching GitHub stats for $REPO..."

# GitHub stats
STARS=$(gh api "repos/$REPO" --jq '.stargazers_count')
FORKS=$(gh api "repos/$REPO" --jq '.forks_count')
OPEN_ISSUES=$(gh api "repos/$REPO" --jq '.open_issues_count')

echo "  Stars: $STARS"
echo "  Forks: $FORKS"
echo "  Open issues: $OPEN_ISSUES"

echo "Fetching crates.io stats..."

# crates.io stats
HOM_BRAIN=$(curl -s "https://crates.io/api/v1/crates/hom-brain" | jq -r '.crate.downloads // 0')
HOM_SHARED=$(curl -s "https://crates.io/api/v1/crates/hom-shared" | jq -r '.crate.downloads // 0')
HOM_INGRESS=$(curl -s "https://crates.io/api/v1/crates/hom-ingress" | jq -r '.crate.downloads // 0')
TOTAL_DOWNLOADS=$((HOM_BRAIN + HOM_SHARED + HOM_INGRESS))

echo "  hom-brain: $HOM_BRAIN"
echo "  hom-shared: $HOM_SHARED"
echo "  hom-ingress: $HOM_INGRESS"
echo "  Total downloads: $TOTAL_DOWNLOADS"

# Append to CSV
echo "$DATE,$STARS,$FORKS,$OPEN_ISSUES,$HOM_BRAIN,$HOM_SHARED,$HOM_INGRESS,$TOTAL_DOWNLOADS" >> "$CSV"

echo ""
echo "Metrics saved to $CSV"
echo ""
echo "=== Summary ==="
echo "Date: $DATE"
echo "GitHub: $STARS stars, $FORKS forks, $OPEN_ISSUES open issues"
echo "crates.io: $TOTAL_DOWNLOADS total downloads"
