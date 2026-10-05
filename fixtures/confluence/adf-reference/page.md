---
rfluence:
  id: "458790"
  space_key: rfluencete
  parent: "753877"
  version: 5
  url: https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/458790
  labels: [two, words, ünïcode, dash-ok, under_score, upper, comma, label]
---

# rfluence ADF reference

API-created reference page for rfluence. Each section exercises one group of ADF nodes. Sections marked **(editor)** are for content added by hand in the Confluence editor.

```adf
{"type":"extension","attrs":{"extensionKey":"toc","extensionType":"com.atlassian.confluence.macro.core","layout":"default","parameters":{"macroMetadata":{"schemaVersion":{"value":"1"},"title":"Table of Contents"},"macroParams":{}}}}
```

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
<th>

Name

</th>
<th>

Value

</th>
<th>

Notes

</th>
</tr>
<tr>
<td>

pipe in code

</td>
<td>

`a|b`

</td>
<td>

filter `type:a|type:b`

</td>
</tr>
<tr>
<td>

list in cell

</td>
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

<!-- rf: colwidths=120,200,300 numbered header-column -->

Merged cells and a coloured cell:

<table>
<tr>
<th colspan="2">

Merged across two columns

</th>
<th>

C

</th>
</tr>
<tr>
<td rowspan="2">

Merged down two rows

</td>
<td>

B2

</td>
<td style="background-color: #deebff">

C2

</td>
</tr>
<tr>
<td>

B3

</td>
<td>

C3

</td>
</tr>
</table>

Image and nested expand inside a table:

<table>
<tr>
<th>

Image

</th>
<th>

Expand

</th>
</tr>
<tr>
<td>

![](attachments/small.png)<!-- rf: layout=center -->

</td>
<td>

<details><summary>Nested expand</summary>

Hidden in a table cell.

</details>

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

```adf
{"type":"decisionList","content":[{"type":"decisionItem","attrs":{"state":"DECIDED"},"content":[{"type":"text","text":"We use ADF, not storage format"}]}]}
```

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

```adf
{"type":"panel","attrs":{"panelColor":"#eae6ff","panelIcon":":rocket:","panelIconId":"1f680","panelIconText":"🚀","panelType":"custom"},"content":[{"type":"paragraph","content":[{"type":"text","text":"Custom panel with colour and emoji"}]}]}
```

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

**bold** *italic* `code` ~~strike~~ <u>underline</u> H<sub>2</sub>O x<sup>2</sup> <span style="color: #ff5630">red text</span> <span style="background-color: #fedec8">highlighted</span> [***bold italic link***](https://example.com)

Centred paragraph<!-- rf: align=center -->

### Right-aligned heading<!-- rf: align=end -->

Indented paragraph<!-- rf: indent=1 -->

Line one\
line two after a hard break

> A quote. **With bold.**

-----

## Install & Setup (v2.0)

Same-page heading link: [back to Install & Setup](#install--setup-v20). Cross-page heading link: [FAQ on the link test page](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/295349/rfluence+link+API+test#FAQ:-What's-new?)

## Smart links

Inline card: <https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/295349>

<https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/131074><!-- rf: card=block -->

<https://www.youtube.com/watch?v=dQw4w9WgXcQ><!-- rf: card=embed layout=center width=100 -->

## Images and files

![](attachments/striped.png)<!-- rf: layout=center caption="Image with a caption" -->

![Alt text for the striped image](attachments/striped.png)<!-- rf: layout=center width=300 -->

![](attachments/small.png)<!-- rf: layout=center border=2 border-color=#091e4224 -->

[![](attachments/small.png)](https://example.com/linked-image)<!-- rf: layout=center -->

```adf
{"type":"mediaGroup","content":[{"type":"media","attrs":{"collection":"contentId-458790","id":"95a757c6-c9d7-4bb2-9b13-83ecf4cf2d20","type":"file"}}]}
```

## Inline nodes

Emoji: 🎉 skin tone: 👍🏽 flag: 🇳🇿

Status: <span data-adf='{"type":"status","attrs":{"color":"blue","style":"bold","text":"IN PROGRESS"}}'>IN PROGRESS</span> <span data-adf='{"type":"status","attrs":{"color":"green","style":"bold","text":"DONE"}}'>DONE</span> <span data-adf='{"type":"status","attrs":{"color":"red","style":"bold","text":"BLOCKED"}}'>BLOCKED</span>

Mention: <span data-adf='{"type":"mention","attrs":{"id":"712020:05eb3148-b0e1-450d-a60d-67073e3cc131","text":"@Chris Edwards"}}'>\@Chris Edwards</span>

Date: <span data-adf='{"type":"date","attrs":{"timestamp":"1798761600000"}}'>2027-01-01</span>

## Expand and macros

<details><summary>Click to expand</summary>

Hidden content.

```python
print(1)
```

</details>

```adf
{"type":"bodiedExtension","attrs":{"extensionKey":"excerpt","extensionType":"com.atlassian.confluence.macro.core","layout":"default","parameters":{"macroMetadata":{"schemaVersion":{"value":"1"},"title":"Excerpt"},"macroParams":{}}},"content":[{"type":"paragraph","content":[{"type":"text","text":"This paragraph is the page excerpt."}]}]}
```

```adf
{"type":"extension","attrs":{"extensionKey":"children","extensionType":"com.atlassian.confluence.macro.core","layout":"default","parameters":{"macroMetadata":{"schemaVersion":{"value":"2"},"title":"Child pages"},"macroParams":{}}}}
```

## Three-column layout

<!-- rf: columns=33.33,33.33,33.34 -->

Left column

<!-- rf: column -->

Middle column

<!-- rf: column -->

Right column

<!-- rf: end-columns -->

## (editor) Added by hand

Content inserted through the Confluence editor goes below, for comparison with the API-created nodes above.

Line with inline comment

emoji here: :rfluence:

<span data-adf='{"type":"status","attrs":{"color":"blue","style":"mixedCase","text":"blue status"}}'>blue status</span>

another <span data-adf='{"type":"status","attrs":{"color":"#FFF0B3","style":"mixedCase","text":"yellow status"}}'>yellow status</span> inline

<span data-adf='{"type":"mention","attrs":{"id":"712020:05eb3148-b0e1-450d-a60d-67073e3cc131","text":"@Chris Edwards"}}'>\@Chris Edwards</span>

an inline <span data-adf='{"type":"mention","attrs":{"id":"712020:05eb3148-b0e1-450d-a60d-67073e3cc131","text":"@Chris Edwards"}}'>\@Chris Edwards</span> here

<span data-adf='{"type":"date","attrs":{"timestamp":"1791072000000"}}'>2026-10-04</span>

an inline <span data-adf='{"type":"date","attrs":{"timestamp":"1791072000000"}}'>2026-10-04</span> here

| **inside table test** |  |
| --- | --- |
| <span data-adf='{"type":"status","attrs":{"color":"blue","style":"mixedCase","text":"blue in cell"}}'>blue in cell</span> | another <span data-adf='{"type":"status","attrs":{"color":"#FFF0B3","style":"mixedCase","text":"yellow status"}}'>yellow status</span> inline |
| <span data-adf='{"type":"mention","attrs":{"id":"712020:05eb3148-b0e1-450d-a60d-67073e3cc131","text":"@Chris Edwards"}}'>\@Chris Edwards</span> | an inline <span data-adf='{"type":"mention","attrs":{"id":"712020:05eb3148-b0e1-450d-a60d-67073e3cc131","text":"@Chris Edwards"}}'>\@Chris Edwards</span> here |
| <span data-adf='{"type":"date","attrs":{"timestamp":"1791072000000"}}'>2026-10-04</span> | an inline <span data-adf='{"type":"date","attrs":{"timestamp":"1791072000000"}}'>2026-10-04</span> here |

resized table:

| **A** | **B** |
| --- | --- |
| tex tA | text b |

<!-- rf: colwidths=193,566 -->

resized image to 653x653, boarder outline, alt text and link

[![alt text for screen readers](attachments/rfluence-emoji.png)](https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/426008)<!-- rf: layout=align-end width=653 border=2 border-color=#172b4d -->

An inline confluence link <https://tech-accounts11.atlassian.net/wiki/spaces/rfluencete/pages/426008>  copy and pasted

```python
def build_request(base_url: str, page_id: str, include_labels: bool = True, include_properties: bool = True, body_format: str = "atlas_doc_format") -> str:
    return f"{base_url}/wiki/api/v2/pages/{page_id}?body-format={body_format}&include-labels={str(include_labels).lower()}&include-properties={str(include_properties).lower()}"

TOKEN_WITH_NO_SPACES = "aaaaaaaaaabbbbbbbbbbccccccccccddddddddddeeeeeeeeeeffffffffffgggggggggghhhhhhhhhhiiiiiiiiiijjjjjjjjjjkkkkkkkkkkllllllllllmmmmmmmmmm"
```

```shell width=4000
$ rf search "ingestion architecture" --space ENG --limit 3
ID       TITLE                              SPACE  LAST MODIFIED          LABELS                         URL
123456   Ingestion service architecture     ENG    2026-10-03T21:14:09Z   architecture, ai-generated     https://example.atlassian.net/wiki/spaces/ENG/pages/123456
123789   Ingestion runbook: queue failover  ENG    2026-09-28T08:02:41Z   runbook, ingestion, oncall     https://example.atlassian.net/wiki/spaces/ENG/pages/123789
```
