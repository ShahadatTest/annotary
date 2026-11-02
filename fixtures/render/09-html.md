# Raw HTML

Inline HTML: <kbd>Ctrl</kbd>+<kbd>C</kbd>, <abbr title="HyperText">HTML</abbr>, a<sup>2</sup>, H<sub>2</sub>O,
<span style="color:red" class="x" data-l="9-9">span loses style/class/data-*</span>, <br/> break,
then <!-- an inline comment --> and an inline <script>alert("gone")</script> script, plus
<a href="https://x.y" onclick="evil()">handler dropped</a> and <img src="https://x.y/i.png" onerror="evil()" alt="img">.

<details>
<summary>Click to expand</summary>

Hidden **Markdown** paragraph inside details.

</details>

<div align="center">

## Centered heading (Markdown inside an unclosed div)

</div>

<p align="right">raw paragraph tag</p>

<style>
p { color: red }
</style>

<script type="module">
console.log("dropped with content");
</script>

<iframe src="https://evil.example"></iframe>

<img src="img/a.png" alt="local asset in raw HTML">
<img src="img/missing.png" alt="missing in raw HTML">
<video src="vid/c.mp4" controls></video>
<audio controls><source src="snd/a.mp3" type="audio/mpeg"></audio>
<a href="docs/x.pdf">raw link to a PDF asset</a> · <a href="img/missing.png">raw link to a missing asset</a>

<table>
  <tr><th>raw</th><th>table</th></tr>
  <tr><td>1</td><td>2</td></tr>
</table>

<form action="/x"><input type="text"><button>submit</button></form>

<center><font color="red">obsolete tags lose their tags, keep their text</font></center>

<unknown-element attr="1">unknown elements keep their text</unknown-element>

Text after everything, with a stray < b and a > c and && here.
