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

for attempt in 1 2 3; do
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
    git commit -m "Record published changelog for $tag"
  fi
  test "$(git diff --name-only origin/main...HEAD)" = CHANGELOG.md
  git push origin "HEAD:refs/heads/$branch"
  sha=$(git rev-parse HEAD)
  pr=$(gh pr list --repo "$repo" --head "$branch" --base main --state open --json number --jq '.[0].number // empty')
  if [[ -z $pr ]]; then
    printf '## Summary\n\n```diff\n+ Record published %s notes on main\n```\n\nPreserve later pending entries and historical edits.\n\n## Evidence\n\nThe workflow checks that this PR changes only CHANGELOG.md and runs the maintained documentation checks before merging.\n\n## Changelog\n\nThis records existing release notes and needs no new product entry.\n\n## Merge Danger\n\n**Door:** two-way. The documentation update is reversible.\n\n**Blast Radius:** changelog\n' "$tag" > "$work/body"
    gh pr create --repo "$repo" --head "$branch" --base main \
      --title "Record published changelog for $tag" --body-file "$work/body"
    pr=$(gh pr list --repo "$repo" --head "$branch" --base main --state open --json number --jq '.[0].number')
  fi
  previous=$(gh run list --repo "$repo" --workflow release-changelog-check.yml --branch "$branch" --limit 1 --json databaseId --jq '.[0].databaseId // 0')
  gh workflow run release-changelog-check.yml --repo "$repo" --ref "$branch"
  run=
  for poll in {1..30}; do
    run=$(gh run list --repo "$repo" --workflow release-changelog-check.yml --branch "$branch" --commit "$sha" --limit 1 --json databaseId --jq ".[0].databaseId | select(. > $previous)")
    [[ -z $run ]] || break
    sleep 2
  done
  test -n "$run"
  run_url="https://github.com/$repo/actions/runs/$run"
  # workflow_dispatch job checks are not evaluated by pull-request rules.
  # Report the actual guarded workflow result as a commit status instead.
  gh api --method POST "repos/$repo/statuses/$sha" -f context=CI -f state=pending \
    -f target_url="$run_url" -f description='Running release changelog checks' >/dev/null
  state=failure
  if gh run watch "$run" --repo "$repo" --exit-status --interval 10 && \
      result=$(gh run view "$run" --repo "$repo" --json headSha,conclusion --jq '[.headSha, .conclusion] | join(" ")') && \
      [[ $result == "$sha success" ]]; then
    state=success
  fi
  gh api --method POST "repos/$repo/statuses/$sha" -f context=CI -f state="$state" \
    -f target_url="$run_url" -f description='Release changelog scope and maintained checks' >/dev/null
  test "$state" = success
  git fetch origin main
  if ! git merge-base --is-ancestor origin/main HEAD; then continue; fi
  gh pr merge "$pr" --repo "$repo" --squash --match-head-commit "$sha"
  test "$(gh pr view "$pr" --repo "$repo" --json state --jq .state)" = MERGED
  exit 0
done
echo 'Main changed during all three attempts. Retry the changelog workflow.' >&2
exit 1
