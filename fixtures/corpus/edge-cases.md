Text before any heading, so this Document's title cannot come from an H1 on its first line.

#

## 

# A heading with a closing sequence ###

Setext heading, level one
=========================

Setext heading, level two,
across two lines
---

####### Seven hashes is a paragraph, not a heading

#Not a heading either, because there is no space

## The same heading twice

## The same heading twice

## `code` and **bold** and a [link](https://www.example.com) and 🧭 in one heading

###### Level six, the deepest there is

## A very long line

The survey party set out from the harbour at first light with two handhelds, a spare battery for each, a flask of coffee that would be cold by nine, and a printout of last year's map on which somebody had drawn the new diversion in pencil, and they walked north along the shore past the old net sheds and the slipway and the place where the boardwalk stops without warning, recording every bench and tap and gate and stile, noting where the markers had faded and where a landowner had moved a fence a metre to the left without telling anybody, until the path turned inland at the second stream and the fix on the first handheld dropped to twelve metres under the beeches, at which point they stopped for the coffee, compared the two tracks, agreed that the pencil line was wrong by about the width of a tractor, redrew it, and went on through the plantation to the road, where the van was waiting and the driver asked, as the driver always asks, whether it had been worth the walk, and they said, as they always say, that it would be once the data build had run and the line on the map finally matched the ground under somebody else's boots, and then they drove home by the coast road with the windows open and the handhelds charging on the dashboard, and nobody said anything else until the bridge.

An unbroken token, which has no place to wrap at all: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

And a long URL: https://tiles.example.com/v2/tiles/outdoor/15/17512/10187.png?key=pk_live_abcdefghijklmnopqrstuvwxyz&lang=sv&scale=2&format=png&attribution=compact&cache=default

## Lines that end oddly

Two trailing spaces make a break  
right here, and a backslash does too\
right here, and a tab does nothing	
right here. A line with only spaces follows.
   
A non-breaking space sits between these words: 12 km. A zero-width space sits inside thi​s word.
A soft hyphen sits in car­tography. Ampersands: AT&T &amp; &copy; &notanentity; &#x1F5FA; &#0;

## Markup that does not close

An asterisk that opens *and never closes, a backtick ` on its own, a [bracket that goes nowhere,
a ![picture with no target, a <span that never ends, and an emphasis**inside**a word.

A link with no target [like this]() and one with brackets in it [a [nested] label](https://www.example.com/a_(b)).

## Lists that stop being lists

- one
-
- three

1. starts at one
1. still one
7. seven

10) a parenthesis
11) another

* a star
+ a plus
- a dash

- a
  - b
    - c
      - d
        - e
          - f
            - g
              - h, eight levels down

> > > > a quote four levels deep
> > back to two, lazily
continued with no marker at all

## Tables that are nearly tables

a | b
--|--
no outer pipes | still a table

| one column |
|---|
| `a \| b` with an escaped pipe |

| header | only |
|---|---|

| not | a table
no delimiter row

## Code that misbehaves

	indented with a tab, and a # that is no heading

````markdown
A fence inside a longer fence:
```rust
fn main() {}
```
````

```
a fence with no language and a very long line in it, long enough that the block has to scroll sideways rather than push the page wider than the screen on any phone or small laptop that happens to open it
```

~~~ text with spaces after the tildes
tildes work too
~~~

## Thematic breaks and nothing at all

***
___
- - -

[^never-defined] is a footnote reference with no definition, and [^unused] is defined and never used in any way that matters.

[^unused]: This definition has nothing pointing at it from the text above, except the sentence that says so.

The last line of this file has no newline after it, on purpose