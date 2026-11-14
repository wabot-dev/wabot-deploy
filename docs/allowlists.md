# Who may connect

## What it is for

Jorge asked for IP allowlists per service, and then for the same on every
door the node opens itself: its console, each organisation's console, the
registry under the node's name, each project's registry, and SSH. A WAF
decides by what a request says. This decides by where the connection came
from, and that is a question the kernel answers, not the client.

## What was decided, with Jorge

- **An empty list lets nobody in.** "Everyone" is written down: the two
  lines `0.0.0.0/0` and `::/0`. A list that defaulted to open would make
  forgetting the same as deciding.
- **A thing is born with the list it was given.** The forms that create a
  service or a database ask for it, with nothing filled in, so whatever was
  not written down is closed. Every door that existed before allowlists was
  opened to everyone by the migration (`0068_allowlist`), which is what it
  already was. A fresh install gets the same for its console and its
  registry, or `/setup` would be the first page nobody can reach.
- **The real address, never a header.** `X-Forwarded-For` is whatever the
  client wrote. The address is the TCP peer's, which the framework hands
  every request since `wabot-feature-rest-controller` 0.2.3: `ConnectInfo`,
  on the TLS listener as well as the plain one.
- **Three phases.** The address in the framework; the edge's names; the
  kernel's ports — SSH, and every port a service publishes. All three are
  built.

## The doors

| Door | Where | Set by | Enforced |
|---|---|---|---|
| The console | `/node/access` | the node's administrator | the edge, on the node's own names, any path but `/v2` and `/api/network` |
| The registry under the node's name | `/node/access` | the node's administrator | the edge, `/v2` and below on the node's own names |
| An organisation's console | its settings, Domain tab | the node's administrator | the edge, on the organisation's domain |
| A project's registry | the project's settings, once it has a name of its own | an owner of the project | the edge, on that name |
| A service | its settings, Access tab, and the creation form | whoever may deploy in the project | the edge, on every name and redirect of the service, on every node serving them; the kernel, on every port it publishes |
| SSH | `/node/access` | the node's administrator | the kernel, on the ports `sshd -T` reports |

A list is stored as rows, one per address or range, in `allowlist`
(`door`, `subject`, `network`, `note`). No rows is a closed door. Triggers
delete a thing's rows when it goes, because a project takes its services by
`ON DELETE CASCADE` and no caller sees that happen.

## What is always let in

- **The machine itself**: loopback, and every address the node holds. An
  operator with a shell is never locked out of their own console. It also
  keeps the node reaching its own names, which it does.
- **The network's own path, `/api/network`.** Errands, reports and
  enrolment between nodes ride the console's names. A console closed to the
  world would otherwise close the network with it. Those calls authenticate
  themselves.
- **A node of the network, at a registry.** A copy placed on another node
  pulls its image from the owner's registry. The edge admits the address a
  member last called `/api/network` from, so an operator listing their
  office does not stop their own copies starting.

**Not containers.** A container connecting to a name is a client like any
other, and its bridge address is not on the list unless somebody puts it
there. Exempting the bridge was the first plan and was dropped: projects
and organisations are not isolated on the network (see CLAUDE.md, "Things
that are still open"), so an exemption would have let any tenant's
container past the operator's console list. Services reach each other by
the project's internal names, which do not go through the edge.

## Saving a console list that would lock you out

Refused, with the address the change came from, because it is what the
person needs to write and may not know: what their network looks like from
here. Saving from the machine itself is always allowed, which is the way to
close the console to everybody but a shell.

An organisation's console list has no such guard. The operator is not
behind it, and closing it is a thing an operator may mean.

## A name served by another node

The owner sends each edge the list with the name: `allowed` on the `edge`
errand, as the lines comma-joined, `""` for nobody. The edge stores it on
the route (`route.allowed`) and enforces it per request.

Two cases are not the same:

- **The field is absent.** The owner is older than allowlists and said
  nothing, so the name is served as before. `allowed` is `NULL` on the
  route.
- **The field is empty.** The owner said nobody.

An edge older than 0.35.0 (`errand::ALLOWLISTS_SINCE`) would ignore the
field and serve everyone. So **a restricted name is not sent to it**: the
owner sends a withdrawal instead, and the service's Access tab names the
nodes it was withheld from. An open list is still sent, because there is
nothing to ignore. A node whose version is unknown counts as old. Closed,
not open, is what a guess costs.

## No deployment

The edge decides per request, from a table reloaded with its routes. The
lists and the routes are swapped together in one snapshot, so a request
never sees a new route with an old list. Saving a list reloads the table
here, and `tell_edges` queues the errand for the others.

## In the kernel

SSH and published ports are below the edge, so their lists are iptables
rules: one chain, `WABOT-ALLOW`, in `mangle PREROUTING`, jumped to for
new TCP connections to one of the machine's own addresses.

- **One place for two kinds of port.** A published port is a DNAT the CNI
  `portmap` plugin wrote, which goes on through `FORWARD`, or — for a
  database whose primary runs on another node — a listener this process
  holds, which is `INPUT`. In `PREROUTING`, ahead of `nat`, both are a
  packet for a local address on that port. So the forward to a remote
  primary is guarded by the database's own list, on the node it is
  published from, with nothing else to say.
- **Dropped, not refused.** `mangle` cannot `REJECT`. A refused client
  waits out its timeout, as it does behind a cloud provider's security
  group.
- **New connections only.** A connection open when a list changes stays
  open. Measured on the VM: a Postgres connection opened while allowed
  answered an `SSLRequest` sent after its range was removed, and a new one
  from the same address was dropped. It is also what keeps an SSH session
  alive when SSH is closed from inside it.
- **Always let in:** loopback, the overlay (`wabot0`), and any source that
  is one of the machine's own addresses — the same as the edge.
- **Written whole.** `iptables-restore --noflush` with the chain declared
  empties and rewrites it in one transaction. Both families: a list's IPv4
  ranges go to `iptables`, its IPv6 ranges to `ip6tables`, and `0.0.0.0/0`
  is not everyone over IPv6.
- **Kept there.** Before the first container starts, after anything that
  can change a door (a list saved, a deployment), and every ten seconds,
  written again whatever was there: a flush can empty the chain and leave
  the jump. On the VM a flushed chain was back in 3 s and a deleted jump
  in 11.
- **Only where armed.** `serve` and the recovery verb arm it. A test runs
  as root inside the VM, and an unarmed pass is what stops the suite from
  rewriting the node's firewall.
- **Its failure is on the page.** A rule the kernel would not take is
  recorded (`firewall.last_error`) and shown on the node's and the
  service's Access tabs, and `doctor` prints it, with whether the jump is
  in the kernel now.

Between boot and the node starting, the machine has no rules — SSH is open
to whoever its own firewall lets in for those seconds. Containers are not
running yet, so no published port is.

## When a list shuts you out

`wabot-deploy allowlist` on the machine itself:

```sh
wabot-deploy allowlist                   # every list
wabot-deploy allowlist ssh everyone      # SSH open again, now
wabot-deploy allowlist console 203.0.113.5 "198.51.100.0/24 office"
wabot-deploy allowlist registry nobody
```

SSH's rules are written by the verb itself. The console's and the
registry's lists belong to the running node, which reads them again within
ten seconds (4 and 10 on the VM). With SSH closed, the terminal is the
provider's serial console.

## Not yet

- **What an organisation's people set.** An organisation's console list is
  the operator's, because the domain it applies to is.
- **The address on the page.** Only the 403 and the refusal say what
  address a request came from. The pages are rendered by controllers that
  do not see the request.
- **UDP.** Nothing publishes UDP today; the chain matches TCP only.
