# Egress: where a project may go, and which nodes it leaves through

Asked for by Jorge on 2026-11-20: decide which nodes a project goes out
to the internet through, and which destinations it may reach. Until now
every container went out through the node it ran on, to anywhere,
masqueraded by CNI.

Four decisions, his:

- **Leaving through another node is TCP, carried by the node.** The same
  path a name on another node already takes (`docs/network.md`, "A
  project's names reach the nodes it runs on"): the node running the copy
  takes the connection and knocks at the exit node's door, which dials the
  destination. The destination sees the exit node's address. UDP to the
  internet from such a project is dropped rather than leaving through the
  wrong node. A tunnel carrying every protocol was considered and not
  taken. It would need an IPIP tunnel per exit over the overlay, because
  WireGuard picks a peer by destination and only one peer can hold
  `0.0.0.0/0`. It would also mean NAT at both ends, MSS clamping for the
  smaller MTU, and a loose `rp_filter`, and each of those is a thing to
  verify on real nodes.
- **Destinations are addresses and names.** `203.0.113.0/24`,
  `203.0.113.5:443`, `api.stripe.com`, `*.stripe.com:443`. For names, the
  node is the containers' resolver.
- **An empty list lets nothing out.** "Anywhere" is the line `0.0.0.0/0`,
  written down, the way the allowlists of `docs/allowlists.md` write
  "everyone". Every project is born with it, and every existing project
  is given it, so nothing changes until somebody takes it away.
  Containers have no IPv6 address, so `::/0` would be a line that means
  nothing. An IPv6 line is refused with a sentence.
- **No exit answering is no internet.** If the exits exist because a third
  party has their addresses on a list, leaving through another address
  fails there anyway, and silently.

## What a list says

One destination per line, then an optional note, as in an allowlist:

| Line | Means |
|---|---|
| `0.0.0.0/0` | anywhere |
| `203.0.113.0/24 the office` | that range, every port and protocol |
| `203.0.113.5:443` | that address, port 443, TCP and UDP |
| `api.stripe.com` | whatever that name resolves to, every port |
| `*.stripe.com:443` | any name under `stripe.com`, not `stripe.com` itself, port 443 |

A port applies to TCP and UDP both: the line says where, not how.

**What a list does not govern:**

- **The project's own network.** Its containers, its names on other nodes
  (stand-ins), the overlay, and the other projects, which
  `infra::isolation` already decides.
- **The node's own addresses.** A container calling another service's
  public name on the same node reaches the edge without leaving the
  machine, and the edge's allowlist judges it as anybody.
- **The node's own traffic:** pulling images, backups, ACME, the
  network. None of it comes from a container.

## Where it is enforced

**On every node running a copy, at the bridge, before any NAT.** One chain
in `mangle`, `WABOT-EGRESS`, jumped to from the top of `PREROUTING` for
`-i wd-+`. It is written whole by `infra::isolation`, with the other
chains, armed only by `serve` and healed every ten seconds. `mangle`
rather than `filter`, because a connection that leaves through another
node is redirected to this process in `nat`, after which `FORWARD` never
sees it. Before NAT, one chain sees both kinds.

In order:

1. Replies and related packets return. A reply to a client on the
   internet who reached a published port is not the container going out.
2. `10.42.0.0/16` and the node's own addresses return. Those are decided
   elsewhere (above).
3. For a project whose list is not anywhere: DNS returns, because it is
   redirected to the node's resolver right after (below).
4. Each line of each project's list returns, as `-i wd-<n> -d <range>`,
   with `-p tcp|udp --dport` when the line has a port, and the addresses
   the resolver learned for its names likewise.
5. A project whose list is not anywhere ends in `-i wd-<n> -j DROP`.

A project whose list is anywhere writes nothing but its absence: it goes
out as it always has.

## Names: the node answers the project's DNS

For a project whose list is not anywhere, `nat PREROUTING -i wd-<n>`
redirects UDP and TCP port 53 to the node's resolver, whatever server the
container asked. Its `resolv.conf` does not change, and a program that
hard-codes `8.8.8.8` is redirected too. The resolver forwards to the
upstreams the containers' `resolv.conf` names, and reads the answer:

- **A name the list allows** (exactly, or under a `*.` line) is answered
  as the upstream answered. Every IPv4 address in the answer, CNAMEs
  followed, is learned for the project: opened in the kernel with the
  line's port, for the record's TTL and never less than an hour, renewed
  by every answer that names it again. Learned addresses are kept in the
  database (`egress_learned`), so a node that restarts does not close a
  connection a program is about to make to an address it cached.
- **Any other name is `NXDOMAIN`, without being asked**, and the name is
  recorded on the project's Internet tab as refused, with when. A program
  fails at once with "not found" rather than hanging on a connection the
  kernel drops, and the tab says why.
- **Unless the list reaches every address on some port** (`0.0.0.0/0`
  with any port): then every name is asked, and only listed ones open
  anything.

The design said a name whose every address the list already allows would
be answered too, so that `10.0.0.0/8` let `db.corp` resolve. Building it
showed the cost. Learning those addresses means asking upstream, and the
question reaches whoever answers for the name, whatever comes back. A
container that may reach nothing could then send anything it likes as
`<data>.attacker.example`. So a name is asked about only when the list
holds it, or when the project could send the same bytes over a port to
any address anyway. `db.corp` goes on the list.

The source address of a query is a container's, and the bridge it belongs
to names the project, as for a stand-in.

**The limit, written where somebody will look for it:** a name on a CDN
opens the CDN's address, and every other name served from that address
with it. A filter that knows the name a connection is for would have to
read TLS's server name, and would not work for anything that is not TLS
or HTTP. This one works for every protocol, and it is that wide.

A project whose list is anywhere keeps resolving against the upstream
directly. A container's DNS then does not depend on this process, which
restarts on every update.

## Exits

A project names the nodes it leaves through. **None means each copy
leaves through its own node**, which is what every project does today.

- **A node must grant `exit`** before anybody's traffic leaves under its
  address. The abuse complaints land on it, which makes this a decision
  about the machine. So `exit` is off until its operator turns it on, like
  `dns`, the one capability whose silence already means no. The node a
  project belongs to may always be its exit.
- **On a node running a copy that is not an exit:** `nat PREROUTING -i
  wd-<n>` redirects new TCP leaving the project (not the project's own
  network, not the node's addresses, not a private or special range) to
  the exit listener on port 31102.
  `SO_ORIGINAL_DST` gives the destination, and the bridge gives the
  project. The listener knocks at the door of the first exit that answers,
  starting at a different one each time, with the project and the
  destination. UDP leaving such a project is dropped, apart from DNS,
  which the node answers.
- **At the exit's door:** the exit admits a node only if it granted
  `exit`, the caller is a member of the project's directory, and the
  project names this node as an exit, and never to a destination that is
  not the internet (`destinations::NOT_THE_INTERNET`: its own network, its
  link-local metadata service, another project's). It then dials the
  destination from the host, answers `ok`, and the bytes flow.
- **The source decides what may leave; the exit decides who may leave
  through it.** The destination list is enforced before the redirect, on
  the node that knows which names were allowed, so the exit does not judge
  it again.

## How it travels

The project's list and exits are the owner's, administered on its
Settings › Internet tab by whoever administers the project there: the node
console's owners of it, or an organisation's. They travel to every node
holding a copy inside the directory errand, which is already sent to each
of them and recomputed every ten seconds. Exits travel as overlay
addresses, so the door can compare.

`errand::EGRESS_SINCE` is the first version that enforces what it is told.
A node older than that would run the project's copies with no list at
all, so saving a list that is not anywhere, or any exit, is refused while
such a node holds a copy, with a sentence naming it. It is `0.44.0`
because this is meant to ship with the names of phase 2. If 0.44.0 ships
without it, the constant moves to the release that has it.

## Phases

1. **Destinations by address**, the chain, the tab, and the directory
   carrying the list. Built and verified 2026-11-20, below.
2. **Destinations by name**: the resolver, learned addresses, refused
   names on the tab. Built and verified 2026-11-20, below.
3. **Exits**: the `exit` capability, the redirect, the listener, and the
   door's second kind of knock. Built and verified 2026-11-20, below.

Each one is verified on the two VMs before the next.

## Phase 1 — built 2026-11-20

- **The list** is `project.egress` (migration `0081`), parsed by
  `domain::values::destinations`. On the owner it is what Settings ›
  Internet saved. On a node holding copies it is what the directory
  carried. A list the holder cannot read closes the project there, and
  the errand fails with the reason.
- **The chain** is `WABOT-EGRESS`, rendered by `infra::isolation` with the
  other three and jumped to from `mangle PREROUTING -i wd-+`. In phase 1,
  port 53 left for any server, so a closed project still resolved names
  and leaked them through the queries. Phase 2 redirects it.
- **It travels** in `errand::Directory.egress`. The owner sends it again
  when it changes, and right behind a placement, so a node given its first
  copy of a project has the list before the copy can start.
- **Saving refuses** a list that is not anywhere while a node older than
  `EGRESS_SINCE` holds a copy, naming the node.
- **`doctor`** says how many projects are limited.

On the two VMs, `pushes` with `whoami` on both and a database on each,
`other` with `probe` on `wabot-dev`, probed with `nc` from each container's
namespace:

| `pushes`' list | to `1.1.1.1:443` | `1.1.1.1:80` | `8.8.8.8:443` | ping | DNS | its database on the other node | `other` |
|---|---|---|---|---|---|---|---|
| `0.0.0.0/0` | open | open | open | | resolves | | open |
| `1.1.1.1:443`, both nodes | open | refused | refused | | resolves | Postgres answers | open |
| empty, both nodes | refused | | | refused | resolves | Postgres answers | |
| `0.0.0.0/0` again | | | open on both | | | | |

The edge served six requests through the closed project, three from each
copy. A container still reached `wabot-dev`'s own address. `wabot-dev-2`
learned each list from the directory, about ten seconds after it was
saved, and its chain held the same lines.

## Phase 2 — built 2026-11-20

- **A line may be a name**, `api.example.com` or `*.example.com`, with a
  port like any other (`destinations::Target::Name`).
- **The resolver** is `infra::egress::resolver`, on port 31103, UDP and
  TCP, started by `serve` beside the names listener and the door. It
  answers a source on a project's bridge and nobody else, the gateway
  included, and forwards to the upstreams written into the containers'
  `resolv.conf`.
- **`nat`** redirects port 53 from every closed project's bridge to it, in
  `WABOT-PROJECTS-NAMES`.
- **What a name opened** is `egress_learned` (migration `0082`), for the
  longest TTL in the answer and never less than an hour. The kernel is
  told before the answer goes back, and only when something was not open
  already. Every pass renders only what is in date, and the ten-second
  loop forgets the rest. Saving a list closes what a name taken off it
  had opened.
- **Refused names** are `egress_refused`, the latest fifty per project, on
  the tab with how often and when.

On the two VMs, `pushes` limited to `api.github.com:443` and
`example.com`, asked with `dig` from each container's namespace:

| Asked | `wabot-dev` | `wabot-dev-2` |
|---|---|---|
| `api.github.com` | `NOERROR` 140.82.114.6 | `NOERROR` 140.82.114.6 |
| `example.com` | `NOERROR` 104.20.23.154 | |
| `wikipedia.org` | `NXDOMAIN` | `NXDOMAIN` |
| `wikipedia.org @8.8.8.8` | `NXDOMAIN`: redirected | |
| `api.github.com +tcp`, and `@1.1.1.1` | `NOERROR` | |
| `wikipedia.org` from `other` | `NOERROR`, not redirected | |

Connections from the same container on `wabot-dev`:

- 140.82.114.6:443 opened, and :80 stayed refused: its line has a port.
- 104.20.23.154 opened on 80 and 443: its line has none.
- 1.1.1.1:443 stayed refused.

`example.com` resolved to two addresses, and both were opened.
`wabot-dev-2` resolved and opened on its own, from the list the directory
carried. Its whoami still reached `ledger` on `wabot-dev` by name. The tab
listed `wikipedia.org`, `2 ×`, "just now".

The learned rules were still there after a restart, before anything was
asked. Setting `0.0.0.0/0` again left no project rule and no redirect on
either node, and `wikipedia.org` resolved directly.

**Not noticed: one chain emptied by hand.** The heal pass asks whether the
jumps are present, and a full flush or a restarted firewall takes them
away. That was healed in two seconds when `mangle PREROUTING` was flushed.
`iptables -t mangle -F WABOT-EGRESS` empties the chain and leaves the
jump, so it went unnoticed until the plan next changed. This is true of
all four chains, and older than this work.

## Phase 3 — built 2026-11-20

- **`exit` is a capability**, off until it is turned on, like `dns`. A
  node that turns it on still carries only for the nodes it grants it to,
  on each node's own Settings tab among the nodes. The owner learns the
  grant from the next report, as it learns every other grant.
- **The exits** are `project.exits` (migration `0083`), as overlay
  addresses. The owner's Internet tab offers this node and every node that
  lets it name it and runs `EGRESS_SINCE` or later. Choosing any is
  refused while an older node holds a copy.
- **The directory carries them**, and goes to the exits as well as to the
  nodes holding copies. It goes to them even when the project runs on its
  owner alone, which before this was a project with no directory. An exit
  holding no copy learns the project from it, and only if it grants
  `exit` to the owner. A directory naming no node is the only withdrawal,
  so one that lists no service still says who may leave. An exit no longer
  chosen is told there is nothing, forgets the directory, and clears the
  exits.
- **On a node that is not an exit**, `infra::isolation` writes, for the
  project's bridge:
  - in `nat`: the DNS redirect, then `RETURN` for every range in
    `destinations::NOT_THE_INTERNET` and for the node's own addresses,
    then every other TCP connection to 31102;
  - in `mangle`, before the list: anything that is not TCP is dropped,
    apart from DNS.

  `mangle` comes before `nat`, so the list is judged against the
  destination the container asked for.
- **The exit listener** is `infra::egress::exit`, on 31102. It reads the
  destination with `SO_ORIGINAL_DST` and the project from the bridge. It
  knocks with `Leave { project, to }` at each exit's door in turn,
  starting one further along each time, and gives up when none opens.
- **The door** tells the two kinds of knock apart (`reach::Asked`). It
  admits a `Leave` only if all of these hold:
  - this node is one of the project's exits;
  - it grants `exit` to the owner, unless it is the owner;
  - the caller is a member of the project's directory;
  - the destination is the internet.

  It then dials from the host.
- **`doctor`** says how many projects leave through chosen nodes.

On the two VMs, with `exit` turned on and granted on `wabot-dev-2`,
probed from each container's namespace with an HTTP request. A plain
connect is not proof: the redirect lands it on the node's own listener,
which accepts it whether an exit answers or not.

| `pushes`' exits, list | from `wabot-dev` | from `wabot-dev-2` |
|---|---|---|
| none, `0.0.0.0/0` | both answer, directly | answers, directly |
| `wabot-dev-2`, `0.0.0.0/0` | 1.1.1.1:80 `301` and example.com `200`, dialed by `wabot-dev-2` from 192.168.104.3; `wabot-dev` held only the door connection | answers, directly: it is the exit |
| `wabot-dev-2`, `1.1.1.1:80` and `example.com:80` | those two answer; 1.0.0.1 and neverssl.com do not | the same |
| `wabot-dev-2` with its node stopped | nothing answers, and nothing leaves `wabot-dev`: "no exit opened, so the connection was refused" | |
| `wabot-dev`, the owner | answers, directly | answers, dialed by `wabot-dev` through its door; 1.0.0.1 still refused |

With `wabot-dev-2` as the exit:

- UDP to 1.1.1.1:123 and 8.8.8.8:443 was dropped, both packets counted
  on the rule.
- SSH on `wabot-dev-2`'s private address answered from `wabot-dev`
  directly, counted on the `192.168.0.0/16` return.
- DNS resolved.

`other` runs on `wabot-dev` alone. Named `wabot-dev-2` as its exit, it
answered through it: `wabot-dev-2` made the project from the directory,
which listed no service. When `wabot-dev-2` withdrew the grant, its door
refused at once with the reason, and nothing answered. The owner's tab
then said `wabot-dev-2` "no longer carries this node's projects, so
nothing leaves through it". Choosing no exit and turning `exit` off left
no redirect and no rule on either node. `wabot-dev-2` forgot `other`'s
directory.

**What a container sees when no exit answers** is a connection accepted
and then closed, not one refused: the handshake is with its own node's
listener, before any exit was asked. Nothing leaves. The reason is in
the source node's log, and so far only there.

**An exit that holds no copy keeps the project's row** after it is no
longer chosen. It shows in that node's projects with nothing in it.

