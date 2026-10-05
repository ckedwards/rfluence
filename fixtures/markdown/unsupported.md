# Unsupported markdown

Markdown with no Confluence equivalent. `rf check` reports each as an error, and upload refuses it (design.md, "Checking markdown").

## Footnotes

Run the health check[^health] before failing over.

[^health]: The health check lives at `scripts/health.sh`.

## Raw HTML blocks

<div align="center">
  <strong>HTML block</strong>
</div>

<!-- A comment between paragraphs -->

## Inline images

An inline image ![icon](unsupported.assets/icon.png) in a sentence.

## Nested quotes

> Outer quote
>
> > Inner quote

> A quote with an alert inside:
>
> > [!NOTE]
> > Alert nested in a quote.
