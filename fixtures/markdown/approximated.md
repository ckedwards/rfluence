# Approximated markdown

Markdown with a close Confluence equivalent. `rfluence check` warns about each; upload and normalize make the same change, so these still round-trip (design.md, "Checking markdown").

Press <kbd>Ctrl</kbd>+<kbd>R</kbd> to reload, or run <code>make test</code>.

HTML formatting: <b>bold</b>, <i>italic</i>, <s>struck</s> and <ins>inserted</ins>.

An <abbr title="Atlassian Document Format">ADF</abbr> node, and a line<br>break.

![Status icon](approximated.assets/icon.png "Status icon")

> [!WARNING] Before you start
> Back up the database first.

<details open>
<summary>Open by default</summary>

Confluence expands always start closed.

</details>

<details><summary>Written on one line</summary>The whole expand in a single HTML block.</details>

| Command | Notes |
| --- | --- |
| `rfluence check` | <ul><li>Offline</li><li>Exits 1 on errors</li></ul> |

<table><tr><th>Option</th><th>Effect</th></tr><tr><td><code>--force</code></td><td>Overwrites remote changes<br>Use with care</td></tr></table>
