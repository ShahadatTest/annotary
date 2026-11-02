# Basics

Paragraph with **strong**, *emphasis*, ***both***, ~~strikethrough~~, `code with <angle> & amp`,
a soft break here and a hard break at the end of this line  
after two spaces, and a backslash hard break\
here.

Setext heading
==============

Another setext
--------------

## Links

Inline [link](https://example.com/path?a=1&b=2 "Title here"), reference [ref][r1], collapsed
[r1][], shortcut [r1], autolink <https://auto.example/x?y=1>, email autolink <me@example.com>,
fragment [to basics](#basics), and a bare URL https://bare.example.com/a_(b)_c. Bare www.example.org,
bare email hi.there+tag@example.org. A `javascript:` link: [click](javascript:alert(1)) becomes text.

[r1]: https://ref.example/target "Reference title"

## Entities and escapes

Entities: &copy; &amp; &lt;tag&gt; &#169; &#x1F600; — escapes: \*not emphasis\* \# not a heading,
\`not code\`, back\\slash. Unicode: Ünïcödé, 日本語, emoji 🎉.

Heading ids get de-duplicated:

### Same

### Same

### Same

## Blockquotes

> Quoted paragraph one.
> Still the first paragraph.
>
> Second paragraph in the quote.
>
> > Nested quote.

Thematic breaks:

---

***

___

Final paragraph.
