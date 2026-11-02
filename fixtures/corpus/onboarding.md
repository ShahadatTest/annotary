# Welcome to Fernway Maps

We are eleven people who make maps for organisations too small to have a mapping team. This guide
gets you from a new laptop to your first change in production, usually within your first week. If
something here is wrong, fix it: this page is edited like everything else we write.

> The one rule we ask you to remember: a map is a promise. Somebody will walk, sail or drive on the
> strength of what we draw, so we would rather ship late than ship a path that is not there.
>
> > "I once followed one of our trails into a bog. The trail was real; the data was eleven years
> > old. We fixed the data and I bought new boots."
> >
> > — Tomasz Reed, on his first week

## Before your first day

Most of this is done for you. Check the list when you arrive and tell Ana Lima if anything is
missing.

- [ ] Laptop delivered and set up with disk encryption
- [ ] Accounts created
  - [ ] Source control, with two-factor authentication
  - [ ] Chat, where you will be added to `#general` and `#tiles`
  - [ ] The on-call tool, read-only until your second month
  - [ ] The survey-data drive, for reading field notes
- [ ] A hardware security key posted to you
- [x] Your name added to the [team page](https://www.example.com/team)
- [x] Payroll forms returned to Ana

## Your first day

Nobody expects you to change anything on your first day. Meet people, read things, and set up your
machine in this order:

1. Install the toolchain.
   1. Run the bootstrap script in the `infra` repository.
   2. Check that it finished with `fernway doctor`, which prints one line per tool.
   3. If a tool is missing, the line says what to install. Ask in `#tiles` if it does not.
2. Clone the three repositories you will touch most.
   - `tiles`, the tile service and the renderer's job queue
       - Read its `README` first, then the [cache RFC](rfc-tile-cache.md).
       - Its test suite takes four minutes and needs no network.
   - `styles`, the map styles and the sprite sheets
   - `data-builds`, the nightly data pipeline
3. Render your first tile locally:

   ```bash
   fernway-tiles render --style outdoor --zoom 12 --x 2189 --y 1273 --out /tmp/first.png
   ```

   Open the picture. It should show the Gothenburg archipelago, and a good deal of water.

4. Have lunch with your buddy. It is the most important step on this page.

## Your first week

By Friday you should have a small change in production. It does not matter how small: the point is
to have walked the whole path once, with someone beside you.

### Pick something small

The board has a column called **First change**. Good ones from the past include a misspelt ferry
terminal, a contour label that overlapped a summit name, and a missing attribution link. Choose one
and tell your buddy.

### Make the change

- [ ] Make a branch from `main`
- [ ] Change the code, and add a test if the change can have one
- [ ] Run `make check` until it passes
- [ ] Open a pull request and describe what a reviewer should look at
- [ ] Get one approval, which is all we require
- [ ] Merge it yourself, then watch it deploy

## Your first month

The second and third weeks are for going deeper in one area. Your buddy will suggest one, but you
choose. By the end of the month you should be able to explain to a customer, without looking anything
up, why their map looks the way it does.

1. Week two: shadow support for two mornings.
   - Read the last fifty tickets first.
   - Answer five yourself, with someone checking before you send.
2. Week three: take a small piece of real work from the board.
   - It should need a review from somebody who is not your buddy.
   - It should touch a part of the code you have not seen.
3. Week four: go outside.
   1. Pick a trail we have mapped within a day's travel.
   2. Print the map and walk it.
   3. Write down everything that is wrong, and fix what you can in the data.

> A note from Layla: nobody understands a map until they have been lost using one. The walk is not a
> treat, it is the training, though we do also buy lunch.

## Tools we use

| Tool | What it is for | How you get access |
|---|---|---|
| `fernway` | the command line for everything internal | installed by the bootstrap script |
| `fernway-tiles` | render, publish, roll back and explain tiles | comes with `fernway` |
| `fernway-styles` | lint and preview styles | comes with `fernway` |
| the dashboard | graphs for every region | your account, on your first day |
| the board | what we are working on | your account, on your first day |
| the support tracker | customers' problems | ask Ana in your first week |
| the survey handhelds | field work | sign one out from the shelf by the door |

The `fernway doctor` command checks all of these, and it is the first thing anybody will ask you to
run when something does not work:

```text
$ fernway doctor
ok    toolchain          1.84
ok    source control     signed in
ok    tile renderer      local, 12 fonts
warn  survey handheld    none paired (fine unless you are going outside)
ok    dashboard          reachable
```

## How we talk to customers

Most of our customers are volunteers, or have one technical person who also does six other jobs.
Write to them as you would to a clever friend who has never heard of a tile. Three habits help:

- Say what happened before you say why it happened.
- Give them the next step, even if the next step is "nothing, it is fixed".
- Never say "simply". If it were simple, they would not have written to us.

When you do not know, say so and give a time by which you will. A customer who gets "I don't know
yet, you will hear from me by three" has never once complained.

## How we write things down

| Kind | Where it goes | Who reads it |
|---|---|---|
| A decision and its reasons | an RFC in `docs/rfc/` | everyone, eventually |
| A plan for a few weeks of work | the board, with a short note | the team |
| A customer's problem | the support tracker | support and on-call |
| How something works | next to the code | the next person to change it |
| What happened in an incident | `docs/incidents/` | everyone, on Friday |

We review documents the way we review code, with comments on the exact sentence. Read
[the meeting notes](meeting-notes-2026-03-16.md) from any recent Monday to see the tone: blunt
about the work, kind about the people.

## Words you will hear

The map business has a lot of jargon and we use most of it. The [glossary](glossary-multilingual.md)
has the terms, and also how our customers in Japan, Israel and the Gulf say them, because they will
ask. Three that come up on the first day:

- **Style:** the rules that turn map data into a picture. `outdoor` is our most used one.
- **Tile:** one square of the map at one zoom level. Requests are for tiles.
- **Data build:** the nightly job that turns raw survey and OpenStreetMap data into what the
  renderer reads.

## Time, money and looking after yourself

We work four and a half days a week, and Friday afternoons are for reading, repairs and going
outside. Books are on us, whatever the subject, and we pay for one field trip a year to somewhere we have
mapped, so that you can find out whether we got it right.

Holiday is 30 days and we check that you take it. Expenses under €200 need no approval: send the
receipt to `accounts@example.com` and it will be paid with your next salary.

## Who to ask

| Question | Person | Best way to reach them |
|---|---|---|
| Laptops, accounts, payroll | Ana Lima | chat, or `ana@example.com` |
| Tiles, cache, renderer | Mei Tanaka | `#tiles` |
| Styles, cartography, fonts | Layla Haddad | `#cartography` |
| Data builds, surveys | Tomasz Reed | `#data` |
| Everything else | Rowan Vale | anywhere, any time |

## Being on call

You join the rota in your third month, after shadowing someone for a full week. It is one week in
eleven, and the pager has gone off out of hours about twice a month this year. If you are woken up,
take the next morning off without asking anyone.

> **If you are on call and you do not know what to do,** page Rowan or Mei. That is what they are
> for, and nobody has ever been told off for doing it.

Welcome aboard. We are glad you are here.
