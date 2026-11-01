# Numbering document content

Use CSS counters to label sections or rows without inserting numbers into the
HTML. Reset a counter on its containing element and increment it on each item:

```html
<section class="lines">
  <p class="line">Design review</p>
  <p class="line">Print preparation</p>
</section>
```

```css
.lines { counter-reset: item; }
.line { counter-increment: item; }
.line::before { content: counter(item) ". "; }
.lines::after { content: counter(item) " line items"; }
```

In 2.5.49, the closing label reads `2 line items`. Generated counter text is
resolved in document order, so `::after` includes its children's increments.
Counter mutations in `::before` and `::after` also contribute to later labels
in their counter's scope. `counter-set` changes an existing counter;
`counter-reset` creates a nested counter where appropriate.

`counters(name, ".")` joins the in-scope levels of a nested counter. A style
argument such as `counter(item, upper-roman)` changes the number's formatting.
These expressions can be combined with literal strings and quotation tokens.

The focused checks cover ordinary rendering, fixed compilation, repeated
documents and data-driven reflow. They do not establish complete CSS counter
support or browser raster parity. A new counter created by a pseudo-element on
`display: contents` can still have a different outer scope from Chromium.
Default HTML list-marker behavior and general mutations of implicit list or
footnote numbering retain their existing limitations. See the
[release runbook](release/2.5.49-runbook.md) for the retained checks.
