# Navigation

How somebody finds a thing in the console, and the record of why it is
shaped this way. The frame is `src/shell/layouts/shell.rs`; this is the
reasoning it could not carry in comments.

## What it is for

Jorge, 2026-11-15: *"Se ha complicado bastante y no se entiende muy bien
por qué hay opciones de navegación en el header y otras opciones en el
sidebar que cambian."* The header picked an area — Projects, Network,
Settings — and the sidebar changed with it, so finding something meant
guessing which bar it was in and then which area. Nothing on the screen
stated the rule.

Three answers shaped what replaced it:

- **Whoever runs the machine mostly looks.** At the node's state, at its
  connections with the others, at what is failing. They are the main user
  of a self-hosted node and the administrator of a multi-tenant network.
- **They do not enter an organisation's console.** An organisation is
  managed from outside: its owner, its quota, its domain.
- **A phone is secondary and has to be decent.**

Three proposals were drawn and Jorge chose the one that reorganises
around things. The diagnosis it answers had eleven points; the ones that
decided the shape:

| | What was wrong |
|---|---|
| Two bars | The header chose an area and the sidebar changed entirely. |
| Actions as places | "Invite a node" and "Join a network" sat in a list of pages. |
| Three places to look | Node state in Network, its settings in Settings, the alerts above the projects. |
| "Settings" three times | The installation's, a project's, a service's — one word, three mechanisms. |
| Words with two meanings | Network, Logs, DNS, Access. |
| No pattern for a service | Two buttons, where a node had tabs. |

## The model

**Everything is a thing, and a thing has a page with tabs.** Three
columns, each answering one question and none changing shape:

- **The rail** — what kinds of thing there are: Status, Projects, Nodes,
  Organisations, People. An icon over its word. What an account would be
  turned away from is not listed, so an organisation's administrator has
  Projects and People and a member has Projects.
- **The list** — which ones. The projects (with the services of the one
  the page is in), the nodes, the organisations.
- **The page** — the one somebody chose: its name, then its tabs. What
  it is first, what is particular to it in the middle, **Settings last**.

The header holds what belongs to the session — a field that goes to a
thing by name, the language, the theme, the way out — and no navigation.

| Thing | Tabs |
|---|---|
| Node | Overview · Connections · Version · Settings |
| Project | Overview · DNS · Settings |
| Service | Overview · Logs · Settings |
| Organisation | Overview · Settings |

There is no settings area. "Settings" means the settings of the thing
whose page this is, wherever it appears.

## The map

| Was | Is |
|---|---|
| `/` (the projects, for everybody) | a redirect: `/status` for whoever runs the machine, `/projects` for everybody else |
| — | `/status` |
| `/` | `/projects` |
| `/network` | `/nodes` |
| `/network/invite`, `/network/join` | `/nodes/invite`, `/nodes/join` |
| `/network/<id>` | `/nodes/<id>`, and `/nodes/<id>/connections` |
| — | `/nodes/<id>/version`, `/nodes/<id>/settings` for another node |
| `/node/<tab>` | the same, drawn as this node's Settings tab |
| `/updates`, `/updates/<tag>` | `/node/version`, `/node/version/<tag>` |
| `/settings` | a redirect to `/node` |
| — | `/go?q=` |

Every path that was a page still answers with a redirect, and carries
the refusal it was about to show (`shell::auth::with_error`).

## Decisions, and what each costs

**This machine's settings stay at `/node`.** The object's own address
would be `/nodes/<id>/settings/<tab>`, and it was not taken. `/node`
names nobody, so it is the same on every machine: a concern, a document
and `doctor` can point at `/node/backup` without knowing an id, and
every form on those pages already posts under `/node`. The cost is an
address that does not look like the tab it is drawn as — the frame
lights this node's entry and runs the trail through its name
(`Frame::this_machine`), which is the part somebody sees.

**The forms did not move.** `POST /network/<id>/drain`,
`POST /updates/install` and the rest are where they were. A form's
action is not somewhere anybody is, and renaming forty of them would
have been a change with no page behind it. The cost is a tree where the
pages say `/nodes` and the actions say `/network`.

**The list opens one project.** An operator hosting several
organisations has more services than a column holds, and the ones
somebody jumps between are the ones beside the page they are on.

**No green dot in the list.** It marks a service a copy of which would
not start — read off `replica.last_error`, never asked of the runtime —
and says nothing about the rest. The list is drawn on every page; a
green dot there would be a health check nobody ran.

**A connection is two facts, not a verdict.** What the kernel says about
the tunnel and when that node last reported, side by side
(`node_connections_component`). A handshake older than three minutes is
"Quiet", not "Connected" and not down: a tunnel with nothing to carry
looks exactly like one to a machine that is off, and the report column
is what tells them apart for a node that reports here.

**The list gives its width back.** Below 70rem it is not drawn. The trail
above the title leads to what the list showed. A page could once ask for
the room, and only the log did: it was then the one page where the list
folded away, and reaching it moved everything on the screen (Jorge,
2026-11-21). Its lines scroll sideways inside their own box instead.

**Status says so when nothing is wrong.** The card of what needs
somebody was absent when empty, which was right above a list of
projects. On the page whose question is "is anything wrong", silence is
not an answer.

**Going by name is the server's.** `/go` is a form and a GET. A name
that is one thing's redirects to it; exact names win over names that
contain them, or `orders` could never be reached while `orders-worker`
exists. It reads the same lists the pages do, so nobody is shown a name
they could not have opened. The island adds ⌘K and nothing else.

**One word, one meaning.** A service's "Network" tab is "Ports and
names", its "Logs" tab is "Log retention", and "Access" is "Who may
connect" on a node and on a service. The node's "DNS" is "DNS server":
a project's DNS tab is its zones.

## What it found

- **A node's page had stopped streaming.** When the network's pages
  moved from `/nodes` to `/network`, the stream's route moved with them
  and the island went on opening `/nodes/<id>/live` — a 404 the browser
  retries for ever. The memory meter and the certificate's state stood
  still on a page that looked live. The suite asked the route for its own
  path, so it stayed green. `every_stream_the_script_opens_has_a_route`
  reads the paths out of the script now.
- **`/projects/new` redirected to a page that did not exist.** A node
  that owns nothing sent the form's visitor to `/projects`, which was a
  404 until the list moved there.
- **The stylesheet guard skipped redirects.** Three of the pages it
  named became redirects, and it went on passing while checking half of
  what it listed. A page on that list that answers nothing fails it now.

## Still open

- An organisation's administrator has no page saying what needs them.
  Every concern is node-wide and shown to the operator only.
- Going by name finds things, not their sections: typing "backup" does
  not reach this node's Backup tab.
- A page nested under a tab — a zone, a new service, a release's notes —
  shows the trail and not its thing's tabs.
- The connections table is drawn when the page is, and does not stream.
