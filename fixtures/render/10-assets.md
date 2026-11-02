# Every Asset type

## Images (`<img>`, dimensions from the manifest)

![PNG](img/a.png "A PNG") ![JPG](img/b.jpg) ![JPEG](img/c.jpeg) ![GIF](img/d.gif) ![WebP](img/e.webp)

SVG goes through `<img>` too: ![SVG](img/flow.svg)

## Derivatives (`?derivative=1`)

![HEIC](img/p.heic) ![HEIF](img/q.heif) ![AVIF](img/r.avif) ![BMP](img/s.bmp) ![TIF](img/t.tif) ![TIFF](img/u.tiff)

A HEIC whose derivative could not be produced falls back to the original: ![no derivative](img/v.heic)

## Video and audio via image syntax

![Clip](vid/clip.mp4) ![Mov](vid/clip.mov) ![M4V](vid/clip.m4v) ![WebM](vid/clip.webm)

![MP3](snd/a.mp3) ![M4A](snd/b.m4a) ![AAC](snd/c.aac) ![WAV](snd/d.wav) ![OGG](snd/e.ogg) ![FLAC](snd/f.flac)

## PDF and text via image syntax

![Spec PDF](docs/spec.pdf) ![Notes](docs/notes.txt) ![Linked doc](api-notes.md) ![Markdown](docs/more.markdown)

## Links to Assets stay plain anchors

[API notes](api-notes.md), [more](docs/more.markdown), [notes](docs/notes.txt), [the PDF](docs/spec.pdf),
[the PNG](img/a.png), [a clip](vid/clip.mp4), [audio](snd/a.mp3), [an image link with a fragment](api-notes.md#section?x=1).

## Missing, unsupported, external, odd paths

![Missing image](img/cli-screenshot.png) and [a link to a missing file](img/cli-screenshot.png).
An unsupported type is left as written: [design](design.docx) ![raw](data.bin).
External images stay external: ![Ext](https://example.com/i.png) ![Data](data:image/png;base64,AAAA) ![Bad](javascript:alert(1)).
Odd paths resolve like the scan does: ![dot-slash](./img/a.png) ![parent](../shared/logo.png) ![encoded](img/sp%20ace.png)
![query](img/a.png?v=2#frag) ![absolute](/srv/pics/x.png) ![via reference][pic] [![thumb](img/d.gif)](img/a.png).

[pic]: img/b.jpg "Reference image"
