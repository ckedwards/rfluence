# rfluence skills

Two [Agent Skills](https://agentskills.io) for using `rfluence` from Claude Code (or another agent that supports skills):

| Skill | For | Can change Confluence |
| --- | --- | --- |
| [`confluence-read`](confluence-read/SKILL.md) | searching and reading pages (`rfluence search`, `rfluence fetch --simplified`) | no |
| [`confluence-write`](confluence-write/SKILL.md) | editing, creating and publishing pages (`fetch -o`, `check`, `upload`); [`markdown.md`](confluence-write/markdown.md) describes the markdown | yes, after a dry run and the user's go-ahead |

They're separate so that reading, the common case, loads only a short skill, and the rules for publishing load only when publishing. Their descriptions don't overlap, so the right one is picked.

## Installing

`rfluence` must be [installed](../docs/user/installation.md) and [logged in](../docs/user/authentication.md). Then link the skills into your personal skills directory, so they're available in every project:

```shell
mkdir -p ~/.claude/skills
ln -s "$PWD/skills/confluence-read" ~/.claude/skills/confluence-read
ln -s "$PWD/skills/confluence-write" ~/.claude/skills/confluence-write
```

Or copy them into a project's `.claude/skills/` to share them with that project.

Both skills pre-approve their read-only `rfluence` commands (`allowed-tools`); `rfluence upload` always asks for permission.
