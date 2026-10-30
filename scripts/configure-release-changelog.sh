#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
workflow_permissions=$(gh api "repos/$repo/actions/permissions/workflow" --jq .default_workflow_permissions)
gh api --method PUT "repos/$repo/actions/permissions/workflow" \
  -f default_workflow_permissions="$workflow_permissions" \
  -F can_approve_pull_request_reviews=true >/dev/null
gh api "repos/$repo/actions/permissions/workflow" --jq .can_approve_pull_request_reviews | grep -qx true
