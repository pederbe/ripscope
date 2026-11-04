#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

tag=${1:?Usage: bash scripts/record-release-changelog.sh TAG [check-only]}
mode=${2:-record}
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]
[[ $mode == record || $mode == check-only ]]
repo=${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}
test "$(gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft)" = false
test -z "$(git status --porcelain)"

# Build from trusted main before checking out the generated documentation branch.
work=$(mktemp -d "${TMPDIR:-/var/tmp}/release-changelog.XXXXXX")
trap 'rm -rf "$work"' EXIT
cargo build --quiet --locked --manifest-path tools/release-changelog/Cargo.toml --target-dir "$work/target"
cp "$work/target/debug/release-changelog" "$work/tool"
branch="contrib/release-changelog-$tag"
git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'

git fetch origin main --tags
git checkout --detach origin/main
"$work/tool" record-release "$tag" "$work/plan"
if grep -qx changed=false "$work/plan"; then exit 0; fi
if [[ $mode == check-only ]]; then
  git diff -- CHANGELOG.md
  git restore CHANGELOG.md
  exit 0
fi
cp CHANGELOG.md "$work/CHANGELOG.md"
git restore CHANGELOG.md
if git ls-remote --exit-code --heads origin "$branch" >/dev/null; then
  git fetch origin "$branch"
  git checkout -B "$branch" FETCH_HEAD
  if ! git merge --no-commit --no-ff origin/main; then
    test "$(git diff --name-only --diff-filter=U)" = CHANGELOG.md
  fi
else
  git checkout -B "$branch" origin/main
fi
cp "$work/CHANGELOG.md" CHANGELOG.md
git add CHANGELOG.md
if ! git diff --cached --quiet || git rev-parse -q --verify MERGE_HEAD >/dev/null; then
  git commit -m "Record published changelog for $tag [skip ci]"
fi
test "$(git diff --name-only origin/main...HEAD)" = CHANGELOG.md
git push origin "HEAD:refs/heads/$branch"
pr=$(gh pr list --repo "$repo" --head "$branch" --base main --state open --json number --jq '.[0].number // empty')
if [[ -z $pr ]]; then
  printf '## Summary\n\n```diff\n+ Record published %s notes on main\n```\n\nPreserve later pending entries and historical edits.\n\n## Evidence\n\nThe workflow checks that this PR changes only CHANGELOG.md and runs the maintained documentation checks before merging.\n\n## Changelog\n\nThis records existing release notes and needs no new product entry.\n\n## Merge Danger\n\n**Door:** two-way. The documentation update is reversible.\n\n**Blast Radius:** changelog\n' "$tag" > "$work/body"
  gh pr create --repo "$repo" --head "$branch" --base main \
    --title "Record published changelog for $tag" --body-file "$work/body"
  pr=$(gh pr list --repo "$repo" --head "$branch" --base main --state open --json number --jq '.[0].number')
fi
# The parent workflow checks this exact revision, then reports CI and merges.
# Suppress the duplicate pull_request run, which GITHUB_TOKEN leaves awaiting approval.
printf 'sha=%s\npr=%s\n' "$(git rev-parse HEAD)" "$pr" >> "${GITHUB_OUTPUT:-/dev/stdout}"
