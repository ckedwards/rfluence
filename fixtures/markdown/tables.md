# Tables

Tables GFM can express stay GFM tables. Anything else is an HTML table whose cells hold markdown (design.md, "Element mapping").

## GFM tables

| Service | Owner | Status |
| --- | --- | :---: |
| Gateway | Platform | Live |
| Workers | Ingestion | Beta<br>since March |

## Lists and code in cells

<table>
<tr>
<th>

Step

</th>
<th>

Details

</th>
</tr>
<tr>
<td>

Deploy

</td>
<td>

- Build the image
- Push it to the registry

```shell
make deploy ENV=staging
```

</td>
</tr>
</table>

## Merged and coloured cells

<table>
<tr>
<th colspan="2">

Q1 results

</th>
<th>

Notes

</th>
</tr>
<tr>
<td rowspan="2">

Revenue

</td>
<td style="background-color: #e3fcef">

**Up 12%**

</td>
<td>

Ahead of plan

</td>
</tr>
<tr>
<td style="background-color: #ffebe6">

Down in EMEA

</td>
<td>
</td>
</tr>
</table>

## Header column, images and expands

<table>
<tr>
<th>

Icon

</th>
<td>

![Status icon](tables.assets/icon.png)

</td>
</tr>
<tr>
<th>

Runbook

</th>
<td>

<details><summary>Failover steps</summary>

1. Promote the replica.
2. Update DNS.

</details>

</td>
</tr>
</table>
<!-- rf: colwidths=120,600 numbered -->

## An HTML table as LLMs write it

<table><thead><tr><th>Option</th><th>Effect</th></tr></thead><tbody><tr><td>force</td><td>Overwrites remote changes</td></tr><tr><td>dry run</td><td><ul><li>Prints the plan</li><li>Changes nothing</li></ul></td></tr></tbody></table>

Text after the tables.
