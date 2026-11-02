# Annotary

A service for sharing Markdown documents between humans and agents and commenting on them, with the same standing for both. Content is authored outside the system and uploaded as versions; Annotary owns the metadata layered on top — threads, actions, versions, membership.

## Language

**Document**:
One Markdown file plus its linked assets (images, video, PDFs) and its Annotary metadata, treated as a single unit with a stable identity across versions. Known by a short stable id; shown by its Name. Documents live in one Vault, with no further grouping.
_Avoid_: Artifact, page, file, package, folder

**Name**:
The filename (without `.md`) a Document was uploaded as, recorded on each Version; the Document goes by its latest Version's Name. Names may repeat within a Vault — the id is what identifies a Document, the Name is how people read and, when unambiguous, refer to it. Uploading under a different filename is how a Document is renamed.
_Avoid_: Slug, title (the heading inside the Markdown), key, path

**Version**:
One immutable snapshot of a Document's content — the Markdown plus its Assets — produced each time content is uploaded. Numbered in order per Document and identified by its content; uploading content identical to the current head produces nothing, while uploading content identical to an older Version produces a new Version (noted as the same content — never a revert). Versions are never deleted. Threads and Actions are pinned to a Version.
_Avoid_: Revision, commit, save, revert

**Asset**:
A local file the Document's Markdown refers to by relative path — another Markdown or text file, an image, video, audio, or PDF — that travels with the Document and is identified by its content. A linked Markdown file is an Asset (with its own Assets), not a Document. A referenced file that can't be found is a missing Asset: the Document still uploads, and the gap is shown.
_Avoid_: Attachment, upload, media, blob (for the concept), package

**Deleted**:
A Document the owner has removed from view. Its Versions, Threads, and Actions stay with it and it can be restored; a separate purge removes it for good.
_Avoid_: Trashed, archived, hidden, soft-deleted (in user-facing text)

**Sidecar**:
The `<name>.annotary` file written next to `<name>.md` on pull: the server's full JSON projection of the Document — identity, Versions, base Version, asset manifest, Actions, Threads with Anchors, and the Members they name. Not for humans or agents to read directly (the CLI is the interface); it exists so the data is plain, portable files. On push only its identity, base Version and any per-Document Action declaration are read.
_Avoid_: Metadata file, manifest, index, database (for the concept)

**Vault**:
The container that holds Documents and on which Grants are given. A Tenant starts with one Vault and may create more.
_Avoid_: Container, workspace, folder, repo

**Thread**:
A root Comment about a Subject in a Document (a span, the whole Document, or an asset), plus its Replies. A Thread is either open or resolved, independent of where its Anchor currently points.
_Avoid_: Comment (for the whole thread), conversation, discussion

**Comment**:
One message in a Thread — either the Thread's root or a Reply — written by one Member in Markdown. Deleting a Comment leaves a tombstone; the Thread and its ids stay.
_Avoid_: Note, message, post

**Reply**:
A Comment that answers a Thread's root. Replies belong to the Thread, not to a Version.
_Avoid_: Response, child comment, answer

**Mention**:
An `@handle` in a Comment naming a Member. Resolved to that Member when the Comment is posted; it makes the Thread show up in that Member's mentions and can wake their wait.
_Avoid_: Tag, ping, notification

**Handle**:
A Member's short name within one Tenant, unique there forever and used in Mentions (e.g. `@rowan`, `@scribe`). It lives on the Membership.
_Avoid_: Username, login, nick, id

**Anchor**:
Where a Thread points in a Document. It has an original placement (the Version, span, and quoted text it was created on — frozen forever and used as the Thread's label) and a current placement in the latest Version, plus its placement in every Version between. Only span and asset Threads have Anchors.
_Avoid_: Position, location, target, range (for the whole concept)

**Device**:
One place a Member uses annotary: an iPhone, an iPad, or **Web**, which stands for every browser the Member uses. Known to the server per Member; it can be renamed and forgotten (Web can't). Forgetting a Device signs out what was last used from it.
_Avoid_: Client, session, browser (for the concept), install

**Setting**:
One choice a Member makes about how Annotary looks or behaves. A user setting has one value per Member; a device setting has a value per Device plus an All devices value that every Device without its own uses. Settings live on the server, so a new Device or a reinstall gets them back. The server-wide choices an owner makes (password, auto-purge) are not Settings.
_Avoid_: Preference, option, config (for the concept)

**Subject**:
What a Thread is about: a **span** of the Markdown source, the **whole Document**, or an **asset** (identified by its content). A Document-level Thread has no Anchor and never goes orphaned.
_Avoid_: Scope, type, kind (of comment)

**Anchored / Refined / Orphaned**:
The three placement states of an Anchor in a given Version. **Anchored**: the span is found and its text is unchanged (it may have shifted). **Refined**: the span survives but its text changed underneath the comment — grew, shrank, or was edited. **Orphaned**: nothing of the span survives; the Anchor keeps its last valid placement. Orphaned is sticky — a later Version never revives it. All three are independent of whether the Thread is open or resolved.
_Avoid_: Outdated, stale, moved, detached, deleted (as a state)

**Action**:
A named operation a Member takes on a Document's head Version (e.g. Approve, Send back), recorded as an event on that Version. Each Action has an intent — positive, neutral, or negative. Actions belong to the Version they were taken on: a new Version starts with none, so nothing "goes stale". The set of available Actions is declared per Vault (defaults) or per Document (replacing the defaults) and can change over time.
_Avoid_: Button, status change, workflow step, approval state, stale

**Intent**:
The tone of an Action — positive, neutral, or negative. Drives how it is coloured and nothing else.
_Avoid_: Type, color, outcome

**Member**:
A person or an agent — the one who comments, takes Actions, is mentioned, and holds Credentials. Nothing in the model distinguishes people from agents; the only difference is how a Member authenticates. A Member exists once and can hold a Membership in many Tenants.
_Avoid_: Actor, user, bot, integration, participant, account, identity

**Membership**:
A Member's standing in one Tenant: their Handle there, their role (owner or member), and their Grants. Disabled, never deleted — history and attribution stay.
_Avoid_: Seat, account, access, user record

**Owner**:
The role on a Membership that may manage the Tenant: Vaults, Members and Memberships, default Actions, purge, and settings. Any Member can hold it; a Tenant always has at least one.
_Avoid_: Admin, superuser, root, human owner (as if only humans could be)

**Right**:
One of the four things a Membership may be allowed to do in a Vault: read, comment, action, write. Write covers creating Documents, uploading Versions, deleting and restoring, and declaring a Document's Actions.
_Avoid_: Permission, scope, capability, role (for a single right)

**Grant**:
A set of Rights on one Vault — or on every Vault in the Tenant, including future ones — held by a Membership.
_Avoid_: Scope, ACL, policy, share

**Credential**:
How a Member proves who they are: a Token, or a password (later a sign-in). Credentials belong to the Member, not to any one Tenant.
_Avoid_: Login (as a noun), key, secret (for the concept)

**Token**:
A Credential minted for a Member, shown once, used by the CLI. Grants come from the Member's Memberships, not from the Token.
_Avoid_: API key, PAT, scope-bearing token

**Tenant**:
The unit that holds Vaults and Memberships and has at least one Owner. A local server hosts exactly one; the cloud hosts many, and a Member picks which one they are in.
_Avoid_: Org, team, account, instance, workspace
