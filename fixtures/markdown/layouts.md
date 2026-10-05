# Layouts

A page section in two columns, as the Confluence editor makes them.

<!-- rf: columns=50,50 -->

## Overview

The ingestion service receives events and writes them to the **event store**.

- At-least-once delivery
- Horizontal scaling

<!-- rf: column -->

## Status

| Component | State |
| --- | --- |
| Gateway | Live |
| Workers | Beta |

<!-- rf: end-columns -->

Three columns, full width, with an image, a code block and an expand:

<!-- rf: columns=33.33,33.33,33.34 breakout=full-width -->

![Status icon](layouts.assets/icon.png)

<!-- rf: column -->

```shell
rf fetch 123456
```

<!-- rf: column -->

<details><summary>More detail</summary>

Hidden until expanded.

</details>

<!-- rf: end-columns -->

A table with its own `layout=` setting is not a column layout:

| A | B |
| --- | --- |
| 1 | 2 |
<!-- rf: layout=wide -->

Text after the layouts.
