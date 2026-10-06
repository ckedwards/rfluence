# Using rfluence with AI agents

rfluence is designed to be used by AI agents as much as by people: compact output for reading, clear errors, and refusals instead of silent damage. Two [Agent Skills](https://agentskills.io) in [`skills/`](../../skills) teach an agent how to use it:

- **`confluence-read`**: search and read pages (`rfluence search`, `rfluence fetch --simplified`). Read-only.
- **`confluence-write`**: edit, create and publish pages (`fetch -o`, `check`, `upload`). It shows a dry run and asks before every upload, never uses `--force` without asking, and leaves synced blocks alone.

## Install them in Claude Code

From the rfluence repository:

```shell
mkdir -p ~/.claude/skills
ln -s "$PWD/skills/confluence-read" ~/.claude/skills/confluence-read
ln -s "$PWD/skills/confluence-write" ~/.claude/skills/confluence-write
```

Start a new session and ask, for example, "what does our Confluence say about the release process?" or "update the on-call page with the new escalation contact". The agent picks the right skill.

Other agents that support Agent Skills can use the same directories. For an agent without skills, point it at [reading.md](reading.md) and [editing.md](editing.md).

## Tips

- Log in yourself (`rfluence auth login`) before handing work to an agent; the skills tell agents never to handle your token.
- `rfluence check --warnings-are-errors` in CI keeps agent-written docs exactly representable in Confluence.
- For many pages, keep the docs in a repository and publish them with [`upload --config`](publishing-a-docs-tree.md), so changes are reviewed like code.
