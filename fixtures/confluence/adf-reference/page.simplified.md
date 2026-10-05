---
rfluence:
  title: rfluence ADF reference
  url: https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/458790
  space_key: rfluencete
  labels: [two, words, ünïcode, dash-ok, under_score, upper, comma, label]
  updated: 2026-10-04T04:52:53.059Z
  simplified: true
---

# rfluence ADF reference

API-created reference page for rfluence. Each section exercises one group of ADF nodes. Sections marked **(editor)** are for content added by hand in the Confluence editor.

[Table of contents]

## Code blocks

```python
def greet(name: str) -> str:
    return f"Hello, {name}!"
```

```
no language set
second line
```

````markdown
contains ``` backticks and `inline` ticks

line after a blank line
trailing spaces   
````

1. Step with a code block:
   ```shell
   cargo build --release
   ```
2. Step after the code block

## Tables

<table>
<tr>
<th>Name</th>
<th>Value</th>
<th>Notes</th>
</tr>
<tr>
<td>pipe in code</td>
<td>`a|b`</td>
<td>filter `type:a|type:b`</td>
</tr>
<tr>
<td>list in cell</td>
<td>

- first
- second

</td>
<td>

```python
x = 1
```

</td>
</tr>
</table>

Header row and header column, numbered rows, custom column widths:

|  | Q1 | Q2 |
| --- | --- | --- |
| Revenue | 10 | 12 |
| Costs | 7 | 8 |

Merged cells and a coloured cell:

<table>
<tr>
<th colspan="2">Merged across two columns</th>
<th>C</th>
</tr>
<tr>
<td rowspan="2">Merged down two rows</td>
<td>B2</td>
<td>C2</td>
</tr>
<tr>
<td>B3</td>
<td>C3</td>
</tr>
</table>

Image and nested expand inside a table:

<table>
<tr>
<th>Image</th>
<th>Expand</th>
</tr>
<tr>
<td>[image: small.png]</td>
<td>

**Nested expand**

Hidden in a table cell.

</td>
</tr>
</table>

## Lists and tasks

1. Level 1
   - Level 2 bullet
     1. Level 3 numbered
2. Second item

A list starting at 3:

3. third
4. fourth

<!-- end list -->

- [ ] Unchecked task
- [x] Checked task
  - [ ] Nested task

<!-- end list -->

- We use ADF, not storage format

## Panels

> [!NOTE]
> Info panel (GitHub NOTE)

> [!IMPORTANT]
> Note panel (GitHub IMPORTANT?)

> [!TIP]
> Success panel (GitHub TIP?)

> [!WARNING]
> Warning panel (GitHub WARNING)

> [!CAUTION]
> Error panel (GitHub CAUTION)

> Custom panel with colour and emoji

> [!NOTE]
> Panel with a list and code:
>
> - one
> - two
>
> <!-- end list -->
>
> ```shell
> echo hi
> ```

## Text formatting

**bold** *italic* `code` ~~strike~~ underline H2O x2 red text highlighted [***bold italic link***](https://example.com)

Centred paragraph

### Right-aligned heading

Indented paragraph

Line one\
line two after a hard break

> A quote. **With bold.**

-----

## Install & Setup (v2.0)

Same-page heading link: [back to Install & Setup](#install--setup-v20). Cross-page heading link: [FAQ on the link test page](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/295349/rfluence+link+API+test#FAQ:-What's-new?)

## Smart links

Inline card: [rfluence link API test](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/295349)

[rfluence image API test](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/131074)

<https://www.youtube.com/watch?v=dQw4w9WgXcQ>

## Images and files

[image: striped.png] (Image with a caption)

[image: Alt text for the striped image]

[image: small.png]

[image: small.png]

[file: spec.pdf]

## Inline nodes

Emoji: 🎉 skin tone: 👍🏽 flag: 🇳🇿

Status: [IN PROGRESS] [DONE] [BLOCKED]

Mention: @Chris Edwards

Date: 2027-01-01

## Expand and macros

**Click to expand**

Hidden content.

```python
print(1)
```

This paragraph is the page excerpt.

[Child pages]

## Three-column layout

Left column

Middle column

Right column

## (editor) Added by hand

Content inserted through the Confluence editor goes below, for comparison with the API-created nodes above.

Line with inline comment

emoji here: :rfluence:

[blue status]

another [yellow status] inline

@Chris Edwards

an inline @Chris Edwards here

2026-10-04

an inline 2026-10-04 here

| **inside table test** |  |
| --- | --- |
| [blue in cell]  | another [yellow status] inline |
| @Chris Edwards  | an inline @Chris Edwards here |
| 2026-10-04  | an inline 2026-10-04 here |

resized table:

| **A** | **B** |
| --- | --- |
| tex tA | text b |

resized image to 653x653, boarder outline, alt text and link

[image: alt text for screen readers]

An inline confluence link [rfluence emoji API test](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/426008)  copy and pasted

```python
def build_request(base_url: str, page_id: str, include_labels: bool = True, include_properties: bool = True, body_format: str = "atlas_doc_format") -> str:
    return f"{base_url}/wiki/api/v2/pages/{page_id}?body-format={body_format}&include-labels={str(include_labels).lower()}&include-properties={str(include_properties).lower()}"

TOKEN_WITH_NO_SPACES = "aaaaaaaaaabbbbbbbbbbccccccccccddddddddddeeeeeeeeeeffffffffffgggggggggghhhhhhhhhhiiiiiiiiiijjjjjjjjjjkkkkkkkkkkllllllllllmmmmmmmmmm"
```

```shell
$ rf search "ingestion architecture" --space ENG --limit 3
ID       TITLE                              SPACE  LAST MODIFIED          LABELS                         URL
123456   Ingestion service architecture     ENG    2026-10-03T21:14:09Z   architecture, ai-generated     https://example.atlassian.net/wiki/spaces/ENG/pages/123456
123789   Ingestion runbook: queue failover  ENG    2026-09-28T08:02:41Z   runbook, ingestion, oncall     https://example.atlassian.net/wiki/spaces/ENG/pages/123789
```
