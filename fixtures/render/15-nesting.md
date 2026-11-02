# Nesting

> A quote with a list:
>
> - item with `code` and a [link](https://x.y)
>   1. nested ordered inside a bullet inside a quote
>
> ```js
> fenced("in a quote");
> ```
>
> | in | quote |
> |----|-------|
> | 1  | 2     |

1. Ordered with a table:

   | a | b |
   |---|---|
   | c | d |

2. Ordered with a heading (allowed by CommonMark):

   ## Heading in a list item

3. Ordered with a task list:
   - [x] done
   - [ ] open

- Bullet with a blockquote:
  > quoted inside a bullet
  > with **strong**
- Bullet with a math block:
  ```math
  a^2 + b^2 = c^2
  ```
- Bullet with a mermaid fence:
  ```mermaid
  graph LR; A-->B;
  ```

Trailing paragraph.
