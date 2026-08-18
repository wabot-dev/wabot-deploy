# The book of use cases

Every way somebody uses wabot-deploy, in an order where each case can be
tried on the state the last one left. Start at part 1 on a machine
with nothing on it and work down.

**Each case has a number.** Quote it when something is wrong — "9.14 does
not work" is a bug report; "the revoke thing is broken" is a
conversation.

**Each case says what should happen.** Where a case is here *because
something once went wrong*, it also says what the fault looked like, so
a regression is recognisable rather than merely surprising. Those are
marked **↩** and they are the ones worth reading before you click.

**What this book does not do** is describe the product. `README.md` is
for that, `docs/architecture.md` for why it is built this way. This is a
list of things to try and what to expect back.

---

## Index

**Part 1 — A node from nothing** · [1](#1-a-node-from-nothing)
1.1 get the binary · 1.2 check the checksum · 1.3 one static file ·
1.4 preflight · 1.5 install with no domain · 1.6 install with a domain ·
1.7 a domain that does not point here · 1.8 staging · 1.9 the setup token ·
1.10 the first account · 1.11 `doctor` · 1.12 the local CA ·
1.13 re-running install

**Part 2 — Signing in and looking around** · [2](#2-signing-in-and-looking-around)
2.1 sign in · 2.2 the wrong password · 2.3 theme · 2.4 language ·
2.5 the empty overview · 2.6 sign out · 2.7 a locked-out operator

**Part 3 — Running something** · [3](#3-running-something)
3.1 a project · 3.2 a service from a public image · 3.3 the first deploy ·
3.4 logs · 3.5 a service that will not start · 3.6 environment ·
3.7 env history · 3.8 stop · 3.9 deploy again · 3.10 delete ·
3.11 a project with a service in it

**Part 4 — Names and ports** · [4](#4-names-and-ports)
4.1 an HTTPS hostname · 4.2 the certificate for it · 4.3 a hostname that
does not resolve · 4.4 a wildcard · 4.5 a raw TCP port · 4.6 remove a port ·
4.7 two services on one node

**Part 5 — Your own images** · [5](#5-your-own-images)
5.1 a push token · 5.2 `docker login` · 5.3 `docker push` · 5.4 the
release list · 5.5 deploy a release · 5.6 watch a tag · 5.7 deploy a push
automatically · 5.8 revoke a token

**Part 6 — Resources** · [6](#6-resources)
6.1 a memory ceiling · 6.2 the ceiling working · 6.3 a CPU limit ·
6.4 what the node has promised · 6.5 a service that does not fit ·
6.6 the disk card

**Part 7 — Databases** · [7](#7-databases)
7.1 create one · 7.2 the preset · 7.3 reach it by name · 7.4 the long name
and the short one · 7.5 a public certificate · 7.6 a standby beside the
primary · 7.7 the read pool · 7.8 publish it on a port · 7.9 change its
name · 7.10 delete it

**Part 8 — Write-ahead log and recovery** · [8](#8-write-ahead-log-and-recovery)
8.1 turn archiving on · 8.2 the window · 8.3 restore to a moment ·
8.4 a moment outside the window · 8.5 the recovered database

**Part 9 — More than one node** · [9](#9-more-than-one-node)
9.1 mint a token · 9.2 the terms · 9.3 join from the console · 9.4 join
from a terminal · 9.5 what each node knows · 9.6 grant `host` ·
9.7 place a copy elsewhere · 9.8 the copies card · 9.9 grant `edge` ·
9.10 who answers for a name · 9.11 traffic by turn · 9.12 stop a service
that runs elsewhere · 9.13 evict · 9.14 revoke `host` · 9.15 revoke
`edge` · 9.16 forget a node · 9.17 a standby on another node

**Part 10 — People** · [10](#10-people)
10.1 invite · 10.2 accept · 10.3 roles · 10.4 project membership ·
10.5 remove somebody · 10.6 revoke an invitation

**Part 11 — Keeping it alive** · [11](#11-keeping-it-alive)
11.1 the attention page · 11.2 a dead upstream · 11.3 a standby that
stopped following · 11.4 check for an update · 11.5 install it ·
11.6 the node page live

**Part 12 — Backup and restore** · [12](#12-backup-and-restore)
12.1 a local backup · 12.2 over SSH · 12.3 to S3 · 12.4 a missing tool ·
12.5 an orphan volume · 12.6 restore a node, same node · 12.7 restore a
node, new node · 12.8 restore while running · 12.9 from a remote backup

**Part 13 — Disk and logs** · [13](#13-disk-and-logs)
13.1 `clean` dry · 13.2 `clean --apply` · 13.3 volumes are refused ·
13.4 timestamped logs · 13.5 the cost on the form · 13.6 a log across a
restart · 13.7 everything kept · 13.8 rotation

**Part 14 — Without JavaScript** · [14](#14-without-javascript)
14.1 every page · 14.2 every form · 14.3 the live pages

---

## 1. A node from nothing

A Linux box, root, nothing installed — not even this program. Ubuntu and
Alpine both: one is glibc and systemd, the other musl and OpenRC, and they
fail differently. **x86_64 only** — the release workflow builds one
target, so an arm64 machine has no binary to download and has to be
cross-built for.

**1.1 Get the binary.** From the releases page, which is the whole of the
installation story:

```sh
v=0.11.2   # github.com/wabot-dev/wabot-deploy/releases
base=https://github.com/wabot-dev/wabot-deploy/releases/download/v$v

curl -fsSLO $base/wabot-deploy-$v-x86_64-linux
```
→ One file arrives. No package, no repository to add, nothing to install
first — that is the claim, and this is where it is either true or not.

**1.2 Check it is the file that was published.**
`curl -fsSL $base/wabot-deploy-$v-x86_64-linux.sha256 | sha256sum -c -`
→ `OK`. Do this rather than skip it: the checksum beside the binary is
what lets somebody verify it came from the project rather than from
whoever served the page. Nothing is signed yet, so this is the whole of
the guarantee.

**1.3 It really is one static file.**
`chmod +x` it, then `--version`, then `ldd` it.
→ The version prints and `ldd` finds **no** shared libraries: musl,
statically linked, rustls rather than OpenSSL, no libc to match. The
release refuses to publish a build where `ldd` says otherwise. That is why
the same file runs on Ubuntu and on Alpine.

**1.4 Preflight.** `wabot-deploy doctor` before installing anything.
→ It names the operating system, privileges, architecture, init system,
programs, cgroups, overlayfs and memory. Nothing crashes on a machine
with no node on it.

**1.5 Install with no domain.** `wabot-deploy install`
→ Finishes. Says where the config, data and database are, that there is
no domain and it will serve a self-signed certificate, prints the local
CA path, starts the node, and prints a setup token.
**↩** On a minimal Alpine this used to die at its first download with
`could not run curl: No such file or directory`. It should install
`curl`, `iptables` and `iproute2` itself and say so.

**1.6 Install with a domain.** `wabot-deploy install --domain node.example --email you@example`
on a machine whose DNS already points here.
→ Gets a real certificate. Fails rather than finishing if it cannot,
unless `--allow-self-signed`.

**1.7 A domain that does not point here.** The same, with a name pointing
elsewhere.
→ Refused with which addresses it found and which this node answers at.
It must **not** place an order it cannot pass.

**1.8 Staging.** `--acme-staging`
→ An untrusted certificate, and no spend against the production account.
This is the flag to use while a DNS problem is being worked out.

**1.9 The setup token.** `wabot-deploy setup-token`
→ A fresh one. It works once and expires in 24 hours.

**1.10 The first account.** Open `https://<node>/setup`, paste the token.
→ An account, signed in. The token no longer works.

**1.11 `doctor`.** `wabot-deploy doctor` on the running node.
→ `no problems found`, plus containerd's version, the socket, the ports,
the overlay if there is one, and the migration state. Not translated: it
prints what somebody pastes into an issue.

**1.12 The local CA.** `https://<node>/ca.crt`, or the file the install
named.
→ Downloads. Trusting it stops the browser warning on a node with no
public certificate.

**1.13 Re-running install.** `wabot-deploy install` again.
→ Converges. It does not repeat what is done, and it does not restart
the node for nothing. Every step asks about the thing, not about whether
it ran before.

---

## 2. Signing in and looking around

**2.1 Sign in.** `/sign-in` with the account from 1.7.
**2.2 The wrong password.** → Refused, and it does not say which half was
wrong.
**2.3 Theme.** The toggle. → Light and dark. Every badge, button and
checkbox stays legible in both — **↩** dark mode once had invisible
unchecked checkboxes, near-invisible secondary buttons and white-on-pink
danger buttons. Also try with the browser set to dark and the toggle
never touched.
**2.4 Language.** The toggle beside it. → Spanish, stored on the account.
Commands, hostnames, ids, image names and containerd's state words stay
in English.
**2.5 The empty overview.** `/` with no projects. → Says so; offers to
make one.
**2.6 Sign out.**
**2.7 A locked-out operator.** `wabot-deploy passwd <username>` on the
node. → Sets a new password from the terminal. This is recovery, not a
way to get a session.

---

## 3. Running something

**3.1 A project.** `/projects/new`. → A slug is derived; the page for it
opens.
**3.2 A service from a public image.** `docker.io/library/nginx:alpine`.
→ Created. The image reference is fully qualified — there is no implicit
registry.
**3.3 The first deploy.** → Pulls, starts, the state goes to running with
an address. The page says which node it runs on.
**3.4 Logs.** The logs page. → What the container said. **↩** A plain
service used to be told its output had not been kept — that advice could
not work, and every service keeps a log now.
**3.5 A service that will not start.** Point one at a tag that does not
exist, or a command that exits 1. → The state says so **and the reason
is on the row**, not only in the journal.
**3.6 Environment.** `KEY=value` per line, one `=` splits, later `=` are
part of the value. → Saved, redeployed.
**3.7 Env history.** Change it twice, then revert a revision. → The old
values come back.
**3.8 Stop.** → Containers go, rows stay.
**3.9 Deploy again.** → Comes back.
**3.10 Delete.** → **The name has to be typed.** Containers, volumes and
rows go. Try it with the wrong name and with the field empty, and with
scripting off: `required` is the browser's courtesy and the check that
counts is on the POST.
**↩** There was no confirmation at all — one press and it was gone. The
reasoning recorded for that was "a dialog needs JavaScript and this
console works without it, and the Danger zone heading is the warning":
true of dialogs, false of text fields, and a heading is not a warning.
**3.11 A project with a service in it.** Try to delete the project.
→ Takes every service with it, and needs the project's name typed for the
same reason 3.10 does.

---

## 4. Names and ports

**4.1 An HTTPS hostname.** Add a port with a hostname under the node's
domain. → Routed. `https://<hostname>/` reaches the container.
**4.2 The certificate for it.** → Issued, or the reason it was not, on
the port's row. Three sources: a public authority, this node's own, or
none.
**4.3 A hostname that does not resolve.** → Refused when typed, with what
it resolves to and what this node answers at. It is checked *before* it
is accepted.
**4.4 A wildcard.** Try `*.example.com`. → Refused by name. The resolver
looks names up in a map, so a wildcard would store a certificate never
served.
**4.5 A raw TCP port.** Add one without a hostname. → The node picks the
outside port and says so. Reachable from anywhere a firewall allows.
**4.6 Remove a port.** → The route and the claim go with it.
**4.7 Two services on one node.** Two hostnames, two containers.
→ Both answer, each on its own name.

---

## 5. Your own images

**5.1 A push token.** Project → tokens. → Shown once. It is the password.
**5.2 `docker login`.** The command is on the service page, ready to
paste. → Succeeds against the node's own registry.
**5.3 `docker push`.** The `build` and `push` commands are on the page
too, with the right tag. → Accepted. **↩** The default tag on that
example was once `alpine`, copied from the base image rather than from
what somebody would push.
**5.4 The release list.** → The push appears, with its digest and when.
**5.5 Deploy a release.** → That exact one, not the newest.
**5.6 Watch a tag.** Set `track_tag`. → A push to that tag is recorded.
**5.7 Deploy a push automatically.** Tick it, push again. → It deploys
without anybody clicking. Untick it and the next push waits.
**5.8 Revoke a token.** → The next `docker push` is refused.

---

## 6. Resources

**6.1 A memory ceiling.** Settings → memory. → Written as `memory.max`
with swap off, and `/dev/shm` follows it.
**6.2 The ceiling working.** Give a container less than it needs.
→ Killed by the kernel, and the page says that rather than something
vague.
**6.3 A CPU limit.** → Written as `cpu.max`. Measured in millicores, the
same unit the reading uses.
**6.4 What the node has promised.** The node page. → Capacity, what is
used, and what is *promised* — the sum of the ceilings. A copy with no
ceiling makes the promise a floor, and the card says so.
**6.5 A service that does not fit.** Ask for more than is left.
→ Refused, counted against what this service already holds. **↩** On a
machine that cannot say how much memory it has, this must not refuse
everything: unmeasurable means unenforced.
**6.6 The disk card.** → Per-replica disks and the node's own breakdown.

---

## 7. Databases

**7.1 Create one.** `/projects/:project/databases/new`. → A service, a
volume, a port, an admin user and password shown once.
**7.2 The preset.** Pick a size. → It sets the cgroup ceiling *and* the
engine's arithmetic — `shared_buffers`, and what it expects as cache. 64
MB with the stock 128 MB of shared buffers is a container killed before
it starts.
**7.3 Reach it by name.** From another container in the same project.
→ Resolves. The node writes these names into every container it starts;
nothing in an image is configured.
**7.4 The long name and the short one.** → Both offered *only* where both
are on the certificate. A public authority cannot sign a short name,
because nothing outside the node resolves it.
**7.5 A public certificate.** Choose it. → `sslmode=verify-full` works
from outside. **↩** An order used to carry one identifier, so choosing
this named the primary and not the read pool, and every read failed
against a certificate with three months left.
**7.6 A standby beside the primary.** Raise the copies to 2. → Seeded
with `pg_basebackup`, `streaming` in `pg_stat_replication`, read-only.
**7.7 The read pool.** → The primary's name with `-ro` in its first
label.
**7.8 Publish it on a port.** → Reachable from outside, with the
certificate the name has.
**7.9 Change its name.** → The certificate is reissued and the names
inside every container of the project are rewritten.
**7.10 Delete it.** → Confirmed. The volumes go with it.

---

## 8. Write-ahead log and recovery

**8.1 Turn archiving on.** The node's capabilities. → The databases are
redeployed, because `archive_mode` is a postmaster setting. **↩** The
switch once changed nothing at all, because nothing caused a deployment.
And an *upgrade* must never do this on its own — a node that already
existed keeps what it already did.
**8.2 The window.** → What can be recovered to, and where it starts.
Note that `archive_timeout` is a floor rather than a promise: 60 seconds
measured two to three minutes.
**8.3 Restore to a moment.**
`wabot-deploy restore <database> --to "2026-08-17 14:32" --into orders-recovered`
→ A **new** database beside the original, at that moment. The original is
untouched. Check a row you deleted after that moment is back.
**8.4 A moment outside the window.** → Refused, saying what the window
is. Not attempted.
**8.5 The recovered database.** → Its page says where it came from and
which moment it holds.

---

## 9. More than one node

Two machines. The one that mints is the *authority*.

**9.1 Mint a token.** `/nodes/enrol`. → A token, and a link.
**9.2 The terms.** Open the link on the joining node. → What the minting
node **requires** and what it **offers**, per capability, *before*
anything is spent. Each required one can be ticked or refused. Terms
shown after committing would be a consent screen for a decision already
made.
**9.3 Join from the console.** Spend it. → A row on each side. The
joining node records an authority; the authority allocates an overlay
address and nothing else. **A node's id is its own.**
**9.4 Join from a terminal.** `wabot-deploy join <token>` on a second
machine. → The same, with no screen to show terms to — typing the command
is the consent.
**9.5 What each node knows.** Both `/nodes` pages, and `doctor` on both.
→ The overlay interface, the peer, the handshake age, the byte counters.
A peer that has never shaken hands says so.
**9.6 Grant `host`.** → The other node may be asked to run containers.
**9.7 Place a copy elsewhere.** Service → placement. → Created *on that
node*, not here and moved. An errand is queued; the node collects it and
writes its own rows.
**9.8 The copies card.** → How many, and how many are elsewhere. Not the
address — that belongs on the replica row.
**9.9 Grant `edge`.** → That node may answer for a name.
**9.10 Who answers for a name.** Settings → edges. → Pick the public
nodes. **Being an edge is a row, including for the node that owns the
service** — a private node can own services served from somewhere else.
**9.11 Traffic by turn.** Two copies there, one here. → The node with two
takes twice the requests. The weight *is* the repetition; nothing
computes a ratio. Measured at 1.89×.
**9.12 Stop a service that runs elsewhere.** → The other machines are
told. **↩** `stop` once took down the copies here and said nothing to
the others, so a service the console called stopped went on serving.
**9.13 Evict.** From the node running a copy, throw it off. → The owner
learns from the report, and stops asking.
**9.14 Revoke `host`.** → What it was running is thrown off, at the next
boot if not before. **↩** Three separate faults once left the claim, the
route and a Let's Encrypt order in place after a revocation.
**9.15 Revoke `edge`.** → It stops answering for the name, the claim is
released, the route is deleted and the certificate is no longer ordered.
**A withdrawal needs no permission** — consent is for taking work on,
not for putting it down.
**9.16 Forget a node.** → The relationship goes. The other machine still
believes until it is told or asked.
**9.17 A standby on another node.** → `streaming`, read-only, and
`verify-full` against the database's qualified name works from either.
**↩** The overlay is a star, not a mesh: two nodes enrolled by the same
authority have never heard of each other, so a standby dialling a primary
on a third machine needs the peer to travel in the errand.

---

## 10. People

**10.1 Invite.** `/people/invite`. → A link.
**10.2 Accept.** → An account, with the role invited.
**10.3 Roles.** Administrator, owner, deployer, viewer. → Each can do
exactly what its name says and no more. Try a deployer changing project
settings.
**10.4 Project membership.** Add somebody to one project. → They see that
project and no other.
**10.5 Remove somebody.** → Their sessions stop working.
**10.6 Revoke an invitation.** → The link stops working.

---

## 11. Keeping it alive

**11.1 The attention page.** → Absent when there is nothing. When there
is: a copy that will not start, copies out of rotation, a certificate
that would not issue, an instruction another node refused, and storage
nothing claims. Each with somewhere to go.
**11.2 A dead upstream.** Kill a container behind a name with several
copies. → Out of rotation in about six seconds, and back in on its own
when it answers. Requests keep succeeding throughout.
**11.3 A standby that stopped following.** Break replication. → The page
says `Not following`.
**11.4 Check for an update.** `/updates`. → Releases, with which one is
running.
**11.5 Install it.** → Downloads, verifies the checksum, replaces the
binary, restarts. The previous binary stays at
`/usr/local/bin/wabot-deploy.previous`. **↩** A restart must not touch a
healthy overlay: the interface, its peers and the kernel's sessions
outlive the process, and rebuilding them cost a remote standby its
replication stream for 45–55 seconds. The journal says
`the overlay interface already matches` when it correctly does nothing.
**11.6 The node page live.** → Memory and CPU move. The meter's width
moves with the numbers beside it. **↩** It froze for as long as it had
existed, because the value assigned was not the shape the CSSOM takes.

---

## 12. Backup and restore

**12.1 A local backup.** `wabot-deploy backup --out /root/b`
→ The database by `VACUUM INTO`, volumes, and this node's own images.
Says every volume it skipped and why. Ends by saying a copy on the same
disk protects against nothing.
**12.2 Over SSH.** `--out ssh://[user@]host/path`
→ Uses your own SSH configuration, keys and `Host` aliases. Blobs are
skipped when already there. The local staging copy goes only once the
transfer says it worked.
**12.3 To S3.** `--out s3://bucket/prefix`, with `[backup.s3]` in
`config.toml`. → One listing rather than a request per blob. Works
against S3-compatible endpoints too.
**12.4 A missing tool.** Try SSH on a machine without `rsync`.
→ Refused **instantly**, naming the package, before anything is built.
**↩** It used to `pg_basebackup` every database and then throw the run
away.
**12.5 An orphan volume.** A volume no copy claims. → Skipped and named.
It is data somebody may still want and there is no row to restore it
under.
**12.6 Restore a node, same node.**
`wabot-deploy restore-node --from <backup> --same-node`
→ **Stop the node first, and wait for the process to go.** The machine
becomes that node: id, keys, grants, certificates. HTTPS works
immediately on the restored certificates with no ACME order. Ends by
saying where this node's names point and where this machine goes out
from — and that it cannot decide which is right.
**12.7 Restore a node, new node.** `--new-node` → The data, a fresh
identity, and a re-join.
**12.8 Restore while running.** → Refused, naming the pid and the stop
command for this machine's init. **↩** It used to write the database out
from under the live process; and `--new-node` failed *after* the copy,
leaving the old identity and an error.
**12.9 From a remote backup.** `--from ssh://…` or `--from s3://…`
→ Brought here first, under the same shape, with only the blobs this
backup names.

---

## 13. Disk and logs

**13.1 `clean` dry.** `wabot-deploy clean` → What it *would* remove.
Nothing is removed.
**13.2 `clean --apply`** → Orphan config, hosts and log files; database
copies from updates past the newest three; image records nothing can
name. `df` may not move, and it says why: a removed image record frees
blobs only when nothing else holds them.
**13.3 Volumes are refused.** → Named and left. `--volumes` is a separate
word, because an unclaimed volume is usually a copy that *moved*.
**13.4 Timestamped logs.** Settings → logs. → Every line gets an instant
and which stream it came from. Takes effect at the next deployment.
**13.5 The cost on the form.** → Read it. About 0.3 MB of private memory
per copy and nearer 8 MB in `top`, almost all shared; 28 bytes on every
line, so the same budget holds about a sixth fewer lines; and the reader
sits between the container and its log.
**13.6 A log across a restart.** Kill a container. → What it said before
is still there, with a boundary line at the new run. **↩** It used to
truncate at every start, so a deployment deleted the evidence of why it
had to happen.
**13.7 Everything kept.** The link on the logs page. → The rotated
generations and the live file as one. It says it is not following.
**13.8 Rotation.** Let a log pass 8 MB. → Rotated **at the next container
start**, keeping the beginning. **↩** Rotating while the container runs
leaves it writing into the rotated file and the live one at zero bytes.

---

## 14. Without JavaScript

Turn scripting off entirely. This is the rule, not a nicety: somebody
opens this console when the node is unhealthy.

**14.1 Every page.** → Renders complete.
**14.2 Every form.** → Submits, and the server refuses what it should.
Anything the browser was enforcing was a courtesy; the check that counts
is on the POST.
**14.3 The live pages.** → Render their values once, statically, and say
they are not following rather than showing a label that only appears with
a script.

---

## What is not here, because it does not exist yet

Named so that finding it missing is not a bug report:

- Nothing **searches** logs.
- No **scheduler**: every placement is a row somebody wrote.
- **Promotion is manual.** A node that promoted on its own would produce
  two primaries the moment a partition looked like a death.
- No **audit log**, no **metrics endpoint**, no **login backoff**.
- No **rollback button** for an update.
- Reconcile checks whether a container runs, not whether its port
  mappings match the rows.
