# Issue tracker: GitHub

Issues live in GitHub Issues in `pederbe/ripscope`. Use the `gh`
CLI with `--repo pederbe/ripscope` on issue commands.

## Common operations

- Read a ticket, its labels and discussion:
  `gh issue view <number> --repo pederbe/ripscope --comments`.
- List open tickets:
  `gh issue list --repo pederbe/ripscope --state open --limit 100 --json number,title,labels`.
  Follow pagination when the task requires the complete backlog.
- Create a ticket:
  `gh issue create --repo pederbe/ripscope --title "..." --body-file <path>`.
- Update its body:
  `gh issue edit <number> --repo pederbe/ripscope --body-file <path>`.
- Add a comment:
  `gh issue comment <number> --repo pederbe/ripscope --body-file <path>`.
- Change labels:
  `gh issue edit <number> --repo pederbe/ripscope --add-label "..." --remove-label "..."`.
- Close completed work:
  `gh issue close <number> --repo pederbe/ripscope`.

For multiline bodies and comments, write the exact text to a UTF-8 temporary file
and pass `--body-file`. Keep review, commit and push requirements in `AGENTS.md` as
the completion rules. This configuration grants no additional write permission.

When a skill says to publish a spec or ticket, create a GitHub issue within the
authorised task. When it says to fetch a ticket, read its body, labels and comments.
Use `docs/agents/triage-labels.md` for state labels.

## Parent tickets and blockers

A wayfinding map is one issue labelled `wayfinder:map`. Its child tickets use
`wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling` or `wayfinder:task`.
Use GitHub sub-issues for parent-child relationships and native issue dependencies
for blocking relationships. If unavailable, use a task list in the parent and
`Part of #<parent>` and `Blocked by: #<number>` lines in the child.

Read blocker states before taking a ticket. A ticket is unblocked only when all
its blockers are closed; readiness does not choose the next work. Follow the
maintainer's priorities.

## Pull requests as a triage surface

**PRs as a request surface: no.**
