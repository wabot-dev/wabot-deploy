# The nodes as name servers

## What it is for

On 2026-10-19 teresaasprilla.com went dark without anything on the node
changing. Its zone lived in Linode's DNS Manager, Linode stops serving a
zone once the account has no machine, and the last machine had just been
deleted. The registrar went on delegating to `ns1–5.linode.com`, and all
five answered REFUSED. The site was up the whole time and nobody could
find it.

So a domain should be able to depend on nothing but the nodes: **the nodes
answer DNS for the zones they are given**, and the operator — or an
organisation, for its own domains — writes the records in the console.

## What was decided, with Jorge

- **The nodes are authoritative name servers.** Not a client for somebody
  else's DNS API: that is the dependency that failed.
- **It is a capability, `dns`, and it starts off.** Every other capability
  reads as on until somebody turns it off, because nodes existed before the
  switches. This one opens port 53 to the internet, so turning it on is the
  deliberate act — and a node can grant it to another, which then uses it
  as a name server, like `edge` for names.
- **A zone chooses which nodes answer for it**, one or several, from the
  nodes that offer `dns`. Administered where it was created, carried to the
  others whole, read-only and evictable there — the shape services have.
- **The name servers are the nodes' own names** (`node-0.network…`), so the
  registrar needs no glue records. The cost: resolving the domain also
  depends on those names' DNS.
- **Organisations add their own domains** and are told which name servers
  to set at their registrar. They do not choose nodes.
- **First version:** zones, records, choosing nodes, organisations, and a
  check that says whether the registrar really points here. Records made
  for services by themselves, importing a zone file and wildcard
  certificates come later.

## Phase 1: this node answers for its own zones

- **`hickory-server` 0.26**, authoritative half only: no resolver, no
  DNSSEC, no TLS — nothing that brings a C toolchain into the build. A
  catalog of in-memory zones, built from the rows and swapped whole on every
  change (`infra::dns::Zones`), like the route table.
- **SOA and NS are written by the node**, never typed: NS from the zone's
  servers, the SOA's primary is the first of them, the serial is
  `zone_serial::next` — the clock, or one more than before.
- **REFUSED outside its zones, no recursion, no AXFR.** An open resolver is
  an amplifier anybody can aim. `ANY` is answered minimally by hickory.
- **A share per source over UDP**: twenty a second, bursts of forty. A
  spoofed source is the whole of an amplification attack and only UDP can be
  spoofed; a real resolver that is cut off retries over TCP.
- **It binds the addresses the node holds, never `0.0.0.0`.** On Debian
  `systemd-resolved` holds `127.0.0.53:53` and `127.0.0.54:53`, measured on
  node-0. `[dns] listen` overrides, `[dns] port` (or
  `WABOT_DEPLOY_DNS_PORT`) moves it for a development node.
- **Port 53 is open only while it is needed**: the node provides `dns` and
  answers for at least one zone. The listener follows both without a
  restart — the `dns` switch and every zone change wake it.
- **A zone is addressed by its domain** in the console: `/dns/example.com`.
  The name is unique on a node, and an id in the breadcrumb is nobody's
  name.
- **TXT is cut into 255-byte strings** rather than refused, because a DKIM
  key is longer than one, and quotes pasted from a provider are dropped.
  CAA is `issue` and `issuewild`; `iodef` is refused.
- **Invariants beside the `INSERT`**: one zone per domain on a node; a
  CNAME stands alone at its name; no CNAME at the apex; an owner cannot
  reach into another owner's zone; an organisation cannot take the node's
  own name or another organisation's console domain.
- **`doctor` asks the thing**: a SOA query to each address the listener
  binds, and the serial it hears back.

### What the VM said

Measured on the Alpine VM node, with the console driven with scripting off:

- Off by default, and nothing on port 53 until a zone exists — then
  `192.168.5.15:53` over UDP and TCP, and still nothing on loopback.
- `aa` on every answer for the zone, no `ra`; NXDOMAIN carries the SOA;
  `google.com` is REFUSED.
- A 476-character DKIM key: truncated over plain UDP, whole over TCP.
- Turning `dns` off closed the port and turning it on opened it again, in
  the same process.
- A pasted `"v=spf1 include:mailgun.org ~all"` lost its quotes, and a
  record typed as `k1._domainkey.mg.teresaasprilla.com` was stored as
  `k1._domainkey.mg`.

## Phase 2: the zone travels to the nodes chosen for it

- **`errand::Kind::Zone` carries the whole zone**, every time: records,
  serial, and the name servers **by name**, because the other nodes that
  answer for it may be machines the receiver has never heard of. A
  separate `withdrawn` rather than an empty list, because a zone with no
  records is still a zone.
- **`dispatch::tell_servers` is `tell_edges` again**: rebuilt from the
  rows, `queue_if_changed` on `zone:{id}`, never to this node, never to a
  node outside `candidates_for(Dns)`, and a withdrawal to every node taken
  off. Sent after every change and once at boot.
- **On the receiving node, `serve_zone` claims the domain**: refused when
  that node has its own zone for it or holds it for somebody else. The
  copy is read-only there and evictable — and evicting is one-way, the
  owner sends it again on its next change unless the grant goes.
- **A withdrawal needs no permission**, the rule edges learned: revoking a
  grant must not block the message that cleans up after it.
- **Revoking `dns` takes the zone off the air at once.** What a node
  serves is computed asking the grant every time, the listener refreshes
  it every minute, and the rows go at boot (`forget_ungranted`), beside
  `release_ungranted`.
- **The name servers are only the nodes that answer**: this one and the
  peers still granting `dns`. A chosen node that withdrew is neither sent
  the zone nor listed in its NS — resolvers sent there would hear nothing.
- **The zone's page offers the nodes that agreed**, a disabled box for one
  that did not, and what each was last sent: waiting to be collected,
  answering, told to stop, or the refusal and its reason. The POST checks
  again; a box is a courtesy.
- The token minting form no longer asks for `dns` by default.

Verified on the VM node only as far as one node goes — the servers card,
saving it, no errand to itself, the minting form. **Two nodes answering
for one zone needs two public nodes on one network**, which no VM can be
(enrolment needs a public certificate) and which does not exist today.

## Phase 3: organisations add their own domains

- **The same bodies, two doors.** Creating a zone, adding and removing a
  record, deleting one and describing one are commands now — `CreateZone`,
  `AddRecord`, `RemoveRecord`, `DeleteZone`, `DescribeZone` — reached by a
  `node_user_` and a `tenant_user_` use case each, the moment a second
  actor needed them.
- **An organisation chooses no servers.** Its domain is answered by this
  node and every other that offers `dns` to it, and its console says which
  name servers to set before a domain is even added. Choosing is the
  operator's, on the same zone, from the machine's console.
- **One organisation cannot learn another's domains.** A zone that is not
  theirs reads as one that does not exist — same sentence, whoever holds
  it — and the overlap rules beside the `INSERT` (phase 1) keep one from
  claiming inside another's, or the node's own name.
- **Two routers, as for people.** `tenant_dns_*` controllers on
  `tenant_routes`; the operator's door for choosing servers is not
  registered there at all, so it is a 404 rather than a refusal somebody
  has to remember.
- The DNS item in the topbar follows `scope_admin`: the operator, and an
  organisation's administrator.

On the VM node: an organisation created with its own domain, its owner
joined through the link, added `acme.example` and a record, and `dig`
answered it; `teresaasprilla.com` and `wabot.test` were refused with their
sentences; the operator's list showed the zone as Acme's.

## Phase 4: whether the world asks these nodes at all

- **Asked of the parent, without recursion.** A resolver remembers, and one
  whose delegation is broken says SERVFAIL and nothing about why. The
  parent zone's own servers say exactly what the registrar wrote.
- **The parent is found by its NS records, not its SOA.** Every zone cut
  has NS records; not every resolver answers a SOA question. The one Lima
  gives a VM answers `com SOA` with an empty NOERROR and `com NS`
  correctly, and the first version said "no zone above
  teresaasprilla.com answered" for that reason. Measured on the VM node.
- **The resolvers are the machine's own**, from `/etc/resolv.conf`, with
  two public ones only when the file names none. The network sits behind
  a trait, so the walk is tested with answers written down.
- **Kept as facts, said in the reader's language**: the status, the
  parent's list, or the reason there is none — five states, *delegated
  here*, *partly*, *points elsewhere*, *not registered*, *not known*.
- **A minute after starting, then every half hour**, for the zones
  administered here; a button asks now. `doctor` prints the last answer
  and counts anything but *delegated here* or *not known* as a problem.

Against the real internet, from the VM node: teresaasprilla.com read
*points elsewhere — ns1, ns2, ns3, ns4.linode.com*, which is exactly the
`.com` referral that day, and the sentence that would have saved an
afternoon. `acme.example` read *not known*, because `.example` does not
exist.

## What is left

- **Two nodes answering for one zone, verified on real machines.** The
  code is tested, and the VM cannot be a second public node.
- Records made for a service's hostname by themselves, importing a zone
  file, and wildcard certificates over DNS-01 — named in the plan and
  deliberately not in this one.

## Known limits

- **One public node is one name server**, and many registrars ask for two.
- IPv4 only, because `lan::held()` is. AAAA records are still served.
- No DNSSEC, no zone transfers, no zone import.
