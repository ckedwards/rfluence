Edge Cases
==========

Markdown variants an LLM or human might write that normalize to one canonical form.

Setext subheading
-----------------

Emphasis with _underscores_ and __double underscores__, *stars* and **double stars**, and ~~strikethrough~~.

Line with a hard break using two spaces  
and one using a backslash\
and a soft
wrap.

+ plus list
+ second item

3. ordered list starting at three
4. next

7) ordered list with a paren
8) next

- loose list item

- with blank lines between

- items

Nested blockquote with an alert inside:

> Outer quote
>
> > [!NOTE]
> > Alert nested in a quote.

~~~python
# tilde fence
print("```")
~~~

````markdown
```js
// fence containing a fence
```
````

    indented code block
    second line

```
fence with no language
```

```text
trailing spaces inside code are content   
    
the line above is only spaces
```

```adf
{"type":"status","attrs":{"text":"IN PROGRESS","color":"blue","localId":"abc-123"}}
```

<div align="center">
  <strong>HTML block</strong>
</div>

Entities: &copy; &amp; &lt;tag&gt; &#x1F600; and escapes: \*not emphasis\*, 1\. not a list, \# not a heading.

Inline code with backticks: `` `code` `` and a pipe outside a table: a | b.

***

Trailing text after a thematic break.
