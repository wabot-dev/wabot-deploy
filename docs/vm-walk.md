# The VM walk of 2026-10-14: what is still open, and how to test it again

Every case in [`testing.md`](testing.md) that one node can run was run on
the VM node (`scripts/vm.sh`), plus organisations, which the book does not
cover yet. Jorge asked for two things: test every feature a VM can reach,
with certificates self-signed rather than ordered; and make sure nothing
the console configures still depends on `config.toml`.

Sixteen commits came out of it, `4a38028` to `5e99f49`. This document is
what they did **not** finish, in enough detail to pick up cold:

1. [Before a release: verify on the real nodes](#1-before-a-release-verify-on-the-real-nodes) —
   what changed under the upgrade path and has only run on the VM.
2. [Open faults](#2-open-faults) — found, not fixed, each with what to
   decide, where the code is, how to reproduce it and what "fixed" looks
   like.
3. [Wording and small things](#3-wording-and-small-things).
4. [Documents that now say something untrue](#4-documents-that-now-say-something-untrue).
5. [What one VM cannot test, and how it could be](#5-what-one-vm-cannot-test-and-how-it-could-be).
6. [Re-testing what was fixed](#6-re-testing-what-was-fixed) — one
   regression check per commit.
7. [The tools](#7-the-tools) — setting the VM up the way this walk did,
   and the traps in it.

Numbers in brackets — (3.5) — are cases in the book.

---

## 1. Before a release: verify on the real nodes

These commits change what a node does **when it is upgraded**, and the VM
was installed fresh by the same binary. Only `node-1.tobaw.shop` and the
private node have the history that can break them. Do this with
`scripts/cross.sh` on both, in this order, before tagging.

### 1.1 The seed: the domain and the ACME switch move into the database

`settings::seed` runs at the first `serve` of the new binary. It writes
`node.domain` and `acme.enabled` **only where no row exists**, from
`config.toml` and the environment. A wrong seed is a node that silently
starts — or stops — ordering certificates.

Before deploying, on each node, write down what it is now:

```sh
ssh root@node-1.tobaw.shop grep -nE '"^(domain|disabled)"' /etc/wabot-deploy/config.toml
# single quotes in SQL: SQLite reads "node.domain" as a column name
ssh root@node-1.tobaw.shop sqlite3 /var/lib/wabot-deploy/db/node.db <<'SQL'
select key, value from setting where key in ('node.domain', 'acme.enabled', 'wal.archiving');
SQL
# the private node has no sqlite3: read the file, and `doctor`
ssh -J root@node-1.tobaw.shop root@10.42.0.2 grep -nE '"^(domain|disabled)"' /etc/wabot-deploy/config.toml
```

After `cross.sh`, on each:

```sh
wabot-deploy doctor | sed -n '/^configuration/,/^database/p;/^acme/,/^network/p'
```

Expected:

- **node-1** (public, ACME in use): `acme` shows the directory and the
  contact, **not** `off`. The domain under `network` is the one it had.
  `configuration` may show a `seed node.domain = …` line — that is the
  file's old value being reported, not a problem.
- **private node** (installed `--self-signed`): `acme  off — …`. If it
  says anything else, the file did not carry `disabled = true` and the
  node is about to order certificates for a private name: turn the switch
  off from its Node → Domain page at once.
- Both: the certificates section lists the same certificates as before.

Then the half that matters to Jorge's question: edit the file and watch
nothing happen.

```sh
cp /etc/wabot-deploy/config.toml /root/config.before
sed -i 's/^\[node\]$/[node]\ndomain = "not-this.example"/' /etc/wabot-deploy/config.toml
rc-service wabot-deploy restart
wabot-deploy doctor | grep -A2 '^  seed'     # reported as a seed
# the node still answers to its own name, and the console still says so
cp /root/config.before /etc/wabot-deploy/config.toml && rc-service wabot-deploy restart
```

### 1.2 Migrations `0058` and `0059`

- **`0058`** deletes `wal.archiving = off` only when it was written by
  `0040` (whole-second timestamp) **and no managed database exists**.
  node-1 has a Postgres primary, so it must keep `off`. Check
  `select value, updated_at from setting where key = 'wal.archiving'`
  before and after: same value. If it flipped to absent (on), every
  database on the node redeploys with archiving at its next deployment.
- **`0059`** deletes `service_edge` rows whose hostname no port of the
  service has. On a network this is the table `tell_edges` reads, so run
  what it would delete **before** upgrading node-1:

  ```sql
  select e.service_id, e.hostname, e.node_id from service_edge e
  where not exists (select 1 from port p
                    where p.service_id = e.service_id and p.hostname = e.hostname);
  ```

  Every row listed should be a name nobody serves any more. If one is a
  name that *is* served — a database's public name, a name another node
  answers for — stop: there is a way to put a row there without a port
  that the walk did not see, and `0059` must not ship as it is.

### 1.3 Every registry write goes through the new `content::commit`

`761fab1` rewrote it: a megabyte at a time, on one stream, resuming a
half-written transaction. Every `docker push` to every node uses it.

```sh
# from a machine with docker, against node-1, a project and its push token
docker pull docker.io/library/postgres:17          # several layers over 16 MB
docker tag docker.io/library/postgres:17 node-1.tobaw.shop/<project>/<service>:big
docker push node-1.tobaw.shop/<project>/<service>:big
```

Expected: the push finishes, the release appears on the service page,
and `ctr -n wabot images ls -q | grep <service>` on the node shows it.
Then interrupt a push half-way (Ctrl-C during a large layer) and push
again: it must finish, not fail with `write @… must occur at current
offset …`.

### 1.4 `install` on an OpenRC node no longer restarts it for nothing

Both test nodes are OpenRC. `wabot-deploy install` twice in a row; the
second must end with `the node is already running this binary.` and
`ps -o pid,etime,args | grep "serve$"` must show the same pid and an
uptime that kept counting. On node-1 a restart costs the remote standby
its WAL stream for ~20 s, which is why this matters.

### 1.5 A database's certificate is now built from `certificate_names`

At its next deployment every self-signed database certificate is reissued
with all six names (primary and `-ro`, qualified, short, project). One
from Let's Encrypt or from a file is kept as it is. On node-1, after any
redeploy of the primary:

```sh
psql "postgresql://<user>:<pass>@<name>-ro.<project>:5432/<db>?sslmode=verify-full&sslrootcert=/etc/wabot/ca.crt"
# from a container in the project; and on the primary
select application_name, state from pg_stat_replication;   -- the remote standby, streaming
```

### 1.6 Images typed without a registry are refused when typed

Only typing changed: what is stored keeps deploying. Before upgrading,
see whether anything stored would now be refused if somebody re-saved it:

```sql
-- the rule `ImageReference::host` applies: a first segment is a registry
-- only when a `/` follows it and it has a dot, a port, or is localhost
select slug, image from service
where image <> ''
  and (instr(image, '/') = 0
       or (instr(substr(image, 1, instr(image, '/') - 1), '.') = 0
           and instr(substr(image, 1, instr(image, '/') - 1), ':') = 0
           and substr(image, 1, instr(image, '/') - 1) <> 'localhost'));
```

Checked against `nginx:alpine`, `library/nginx` and `bitnami/redis:7`
(listed) and `docker.io/…`, `localhost/…`, `host:5000/…` (not).

Those rows deploy today only if containerd resolves them, which it does
not (that was the bug); they are worth fixing by hand on the image tab.

---

## 2. Open faults

### 2.1 A container that dies is never started again

**What happens.** `kill -9` a service's process, or let it crash or be
OOM-killed: it stays `STOPPED` until somebody presses deploy or the node
restarts. A standby that dies the same way reads `Not running` on its
row and is on no attention card.

**Why.** `Deployer::reconcile` (`src/infra/deploy/mod.rs`) runs once, as
the first thing the `reconcile` service in `src/cli/serve.rs` does, and
then waits for shutdown. Nothing watches task exits. Failed
*deployments* are retried, because they are jobs; a container that
started fine and then died is not a job.

**What to decide.** This is a restart policy, and it has three questions
the code cannot answer for itself:

1. **Which exits restart.** A container that exits `0` may be a one-shot
   (busybox in the walk) — restarting it every few seconds is a loop. A
   reasonable rule: restart any exit while `desired_state = Running`,
   with backoff; or restart only non-zero exits and OOM kills.
2. **Backoff.** Exponential per replica (e.g. 2 s doubling to 5 min),
   reset after it has run for a while (e.g. 10 min). Without it a crash
   loop is a CPU loop and a log that fills.
3. **Where the reason goes.** Each restart should put
   `exited N, restarted K times` on `replica.last_error`, so the page and
   the attention card ("A copy will not start") say it — *errors are
   values somebody can act on*.

**Where to build it.** Either a periodic pass that does only the
`Observed::Stopped | Absent` arm of `reconcile` (cheap: one `Tasks.Get`
per replica here), or containerd's task-exit events
(`EventsClient::subscribe` with a `/tasks/exit` filter), which react at
once but need the periodic pass anyway for a node that missed an event.
Recommendation: the periodic pass first, every 10 s, with per-replica
backoff kept in memory.

**Reproduce on the VM.**

```sh
scripts/vm.sh root ctr -n wabot t kill -s KILL coffee-store.web
scripts/vm.sh root ctr -n wabot t ls          # STOPPED, and it stays so
```

**Fixed looks like.** Back to `RUNNING` within the backoff, a boundary
line at the new run in its log (13.6 of the book assumes this), the
service page saying it was restarted and why; a busybox-style one-shot
not restarted forever (whatever rule was chosen); a killed standby
`streaming` again in `pg_stat_replication` on the primary.

**Tests.** The backoff arithmetic is pure and belongs in a unit test. The
restart itself touches containerd and is verified on the VM.

### 2.2 A service created under an organisation's quota runs with no ceiling

**What happens.** Organisation with a quota of 64 MB; its administrator
creates a service from the tenant console. It is created and deployed
with **no memory ceiling**, so the quota does not bound it. The quota is
checked when a ceiling is changed or a placement is made
(`quotas::admits`, called from `set_service_resources_command` and
`set_service_placement_command`), never at creation. Setting a quota on an
organisation that already has limitless services does not check them
either (`node_user_set_organisation_quota`).

`5e99f49` fixed the half where two limitless services could never be
given ceilings; this half is still open.

**What to decide.**

- (a) The tenant's create form asks for a memory ceiling when the
  organisation has a quota, and the POST refuses without one. Honest;
  one more field on that form.
- (b) A service created under a quota gets the smallest rung that fits
  what is left, and the page says so. Convenient; a ceiling nobody chose.
- (c) The deployment refuses to start a limitless copy under a quota,
  with the sentence the quota already uses. Catches every door, including
  errands, but the service exists and does not run.

Recommendation: (a) for the form, plus (c) as the check that counts,
since a rule only one door enforces is a rule the next door skips.
And setting a quota should say how many services under it have no
ceiling, rather than refusing.

**Reproduce on the VM.**

```sh
# as the node's administrator: an organisation with a domain, a quota, an owner
scripts/vm_console.py submit /organisations '^/organisations$' name="Acme"
#   … set its domain (org.wabot.test in /etc/hosts), quota 64 MB, invite and join an owner
WABOT_VM_AS=org WABOT_VM_HOST=org.wabot.test scripts/vm_console.py \
  submit /projects/new '^/projects$' name=Big
WABOT_VM_AS=org WABOT_VM_HOST=org.wabot.test scripts/vm_console.py \
  submit /projects/big/services/new '/services$' name=Hog image=docker.io/library/nginx:alpine env=
scripts/vm.sh root sh -c 'cat /sys/fs/cgroup/big.hog/memory.max'   # "max": unbounded
```

**Fixed looks like.** Whichever was chosen: no copy under a quota runs
with `memory.max = max`.

### 2.3 A running container no row claims is reported nowhere

**What happens.** After `restore-node` from a backup older than some
services, their containers keep running — no row, no route, holding
memory — and no page, no `doctor` line, no attention card names them.
The files they left are counted as "Storage nothing claims", which
`9043b27` made name the containers, but the *running process* is
invisible. `reconcile`'s comment said "left alone and reported"; it now
says "and not reported", which is true and not good.

**Why.** Nothing lists containerd's containers: `runtime::containers` has
`status`, `labels`, `stop`, `remove` and no `list`.

**To build.** `containers::list(client) -> Vec<String>` over
`ContainersClient::list` in the `wabot` namespace; ignore the one-shot
helpers (`.chown`, `.seed`, `.ask`, `.backup`, `.unpack` as of
`5e99f49` — grep `deploy/mod.rs` for `"{…}.` before trusting the list,
which is exactly the kind of guard that is right until somebody adds an
input); compare with
`Deployer::claimed`. Show the rest in two places: a `doctor` line under
`runtime`, and an attention concern "Containers nothing claims" (Notice,
linking to the node page) with `clean` offering to stop and remove them —
**never automatically**, for the reason `reconcile` gives.

**Reproduce on the VM.** Take a backup, create a service, stop the node,
`restore-node --from <that backup> --same-node --over-my-dead-body`,
start it: the new service's container is still in `ctr -n wabot t ls`.

### 2.4 The node's own domain is checked against itself (book 1.7)

**What happens.** Saving the node's domain on Node → Domain accepted
`elsewhere.wabot.test`, which resolves to `10.99.99.99`. It now says
"resolves to 10.99.99.99" instead of "resolves here" (`7ac6eb5`) — Jorge
chose the honest sentence. The behaviour is unchanged, and so is the
part with consequences: **`acme::ensure` makes the same comparison**, so
for the node's own name it does not stop an order for a name that points
at another machine. `install --domain` asks no DNS question at all
before the renewal loop does. Book 1.7 promises "it must not place an
order it cannot pass"; for the node's own domain that is not true.

**Why.** `node_user_set_node_domain` and `acme::ensure` (around the
`resolves_here(name, &node_domain)` call) compare a name with the node's
domain — the same name. A node behind NAT cannot see its public address,
which is why nobody wrote the obvious comparison.

**Options, if it is ever reopened.**

- A self-probe: put a random token in the challenge table, fetch
  `http://<name>/.well-known/acme-challenge/<token>` from the node itself,
  and order only if it answers with the token. Proves traffic for the
  name arrives here. Fails on routers without hairpin NAT — a false
  refusal on exactly the home networks a private node lives on, so it
  would need a way past it.
- Compare with the machine's own interface addresses and warn (never
  refuse) when they differ, saying that behind NAT this is expected.

**Reproduce on the VM.** `elsewhere.wabot.test` in the VM's `/etc/hosts`
pointing at `10.99.99.99`; save it as the node's domain. **Do not turn
ACME on to see the order** — see [7.3](#73-traps).

### 2.5 Spanish pages declare `<html lang="en">`

**Why.** `wabot-feature-ui-controller`'s `runtime.rs` builds
`DocumentOptions { lang: None, … }`, and nothing per request can set it;
`render_document` defaults to `"en"`. A framework change: a per-request
`lang` the way `layout::head` sets the title, a test in the framework,
a publish, and then `shell::layouts` setting it from the account's
language. Browsers offer to translate a page from English that is in
Spanish, and a screen reader reads it with English rules.

**Re-test.** Toggle the language and read `<html lang>` on any page:
`scripts/vm_console.py submit / '/language$'` then
`curl -sk -b … https://localhost:9443/ | head -c 60`.

### 2.6 A run that never started gets a "started" line in its log

**What happens.** A service whose image cannot be resolved shows
`── wabot-deploy ── started 18:03:43 UTC ──` for each attempt, though no
container ever started.

**Why.** `deploy_one` calls `logs::resume` (which writes the boundary) and
then `containers::run`, which is where `images::ensure` pulls and fails.

**Fix.** Ensure the image before resuming the log, or have a failed run
append `── could not start: <reason> ──` instead of leaving a bare
"started".

**Reproduce.** Create a service with `docker.io/library/nginx:does-not-exist`
and read its logs page.

### 2.7 The image's `STOPSIGNAL` is ignored

`containers::stop` always sends SIGTERM. `httpd` declares `SIGWINCH`;
`ImageConfig` (`runtime/images.rs`) does not read `StopSignal` at all. It
did not cause a fault on its own in the walk — the stop race in
`8c3de49` did — but a server whose graceful signal is not TERM is killed
after the grace period instead of draining. Read `config.StopSignal`,
map the name to a number, and send that.

### 2.8 Verify: a removed name may stay on the shared local certificate

After a wildcard port was removed, the `localhost` certificate was
reissued still covering `*.wabot.test`. `a13d83b` removed the
`service_edge` rows that kept the name wanted, which may be the whole
cause. **Check:** add a port with `api.wabot.test`, remove it, wait for
the next refresh (or restart), and read the covered names:
`scripts/vm.sh root wabot-deploy doctor | sed -n '/^certificates/,/^acme/p'`.
If `api.wabot.test` is still there, the local certificate's name set is
built from something that is never pruned — look at `certs::own_names`
and the route table it is given.

### 2.9 `doctor` prints a netlink error on a node that was never on an overlay

`  not up: could not read wabot0: Netlink error: … No such device` under
`network`, on a standalone node. Not counted as a problem, and it reads
like one. When the node has no overlay address in its own row, say
`not on an overlay` and stop.

### 2.10 `install` prints every migration at INFO

A first install prints ~57 `applied migration=…` lines before anything
an operator needs. They are `tracing` at INFO from the framework's
migration runner. Either the CLI verbs (not `serve`) start with
`warn` unless `RUST_LOG` says otherwise, or the summary line
(`schema updated migrations=…`) is the only one kept at INFO.

### 2.11 Passwords: no way to change one, and `passwd` keeps the sessions

- There is no form anywhere to change your own password. `094aa26` made
  `passwd` stop promising one. A feature to add: a page under the
  account (current password, new one twice), a use case per actor.
- `wabot-deploy passwd <user>` replaces the password and leaves every
  session of that account valid. As recovery for a locked-out operator
  that is harmless; as the response to a leaked password it is not. Ask
  whether `passwd` should end the account's sessions — and if so, say it
  in its output.

### 2.12 The ACME contact and directory are still only in the file

`acme.email` and `acme.directory` are read from `config.toml` at every
start. They were never on the console, so Jorge's rule about console
settings does not reach them yet. If the switch on Node → Domain should
come with the contact address beside it, they move the same way the
switch did: a row, `settings::seed`, the field private to `infra`.

---

## 3. Wording and small things

Each one is a sentence the machine does not agree with.

| Where | What it says | What is true |
|---|---|---|
| Danger tab of a service or project | "Type the name to confirm: broken", and the error says "the service's name" | It wants the **slug**; the name `Broken` is refused. Accept either, case-insensitively, or say "slug". |
| A viewer POSTing deploy | `?error=no such service` | The viewer can see the service; it is the permission that is missing. Deliberate for foreign services (`service_rest_controller` test comment); for a project member it misleads. |
| Node → Backup, list of backups | "Nothing on this machine" | A CLI `backup --out /root/b1` is on this machine; the list reads only `<data_dir>/backups`. Say "nothing in <path>". |
| S3 refusal on the Backup tab | `backups refused it: GET  → 403: <?xml …>` | Empty path between the two spaces, and the XML whole. Show `<Code>` and `<Message>`. |
| Restore card of a **restored** database | "A backup from 18:22, and no log after it yet" | That backup does not contain this database. The window is anchored on the node's newest backup, not on one holding this volume. |
| Tenant console, port form | "Either add *.<node domain> pointing at this node" | For a tenant the suffix is the organisation's domain; the example above it already uses it. |

---

## 4. Documents that now say something untrue

- `docs/testing.md`
  - **1.4** still describes the right output; it is true again after
    `4a38028`. Leave.
  - **1.7** promises no order for a name that does not point here — see
    [2.4](#24-the-nodes-own-domain-is-checked-against-itself-book-17).
    Add the exception for the node's own domain.
  - **4.4** says a wildcard is refused by name: true again after `7ac6eb5`.
  - **7.2** says the preset fixes 64 MB; the smallest a new database is
    offered is 128 MB now (`39765f4`). Say why.
  - **11.3** "Break replication → Not following": killing the standby
    gives `Not running`, which is honest; `Not following` needs a standby
    that runs and does not stream. Say how to produce each.
  - **13.6** "Kill a container → … a boundary line at the new run" assumes
    a restart nothing performs — see [2.1](#21-a-container-that-dies-is-never-started-again).
  - Add a part for organisations: create, domain, owner, quota, the tenant
    console, isolation — the walk did all of it with nothing to follow.
- `CLAUDE.md` ("One binary cannot serve both nodes. Ubuntu is glibc…")
  describes a pair of test nodes that is now two Alpine machines.
- `CLAUDE.md` "Things that are still open" lists 2.1–2.3 and points
  here; take them off it as they are done.

---

## 5. What one VM cannot test, and how it could be

| Case | Why not | How |
|---|---|---|
| Part 9, more than one node | The control plane trusts only public roots (`network::call::round_trip` uses `with_webpki_roots`), so a self-signed enroller can never be joined. | (a) Join the VM to node-1 as a private member — real, but changes node-1, which holds a Postgres primary. (b) Build it: the join token carries the authority's CA, pinned for the join and the errands, as `to_node` already does over the overlay. Then two VMs (`WABOT_VM=wabot-dev2 scripts/vm.sh up`) can enrol. |
| 11.5, installing an update | Releases are x86_64 only; the VM is aarch64. | An x86_64 node, or a release workflow that also builds aarch64. |
| 1.6, 1.8, 4.2 and 7.5 with a public certificate | ACME is off by design here. | Pebble (Let's Encrypt's test server) in the VM, `[acme] directory = "https://127.0.0.1:14000/dir"` in the file, its root trusted by the node. Never production — see 7.3. |
| 12.7, restore onto a new node | Not run: it mints a new identity, and the VM was kept as one node. | `WABOT_VM=wabot-dev2 scripts/vm.sh up && … deploy`, copy a backup over with `limactl copy`, `restore-node --from … --new-node`. |
| 13.8, rotation at 8 MB | Not run. | Write 8 MB to a container's stdout from inside it: `ctr -n wabot t exec --exec-id x coffee-store.web sh -c 'yes 0123456789 \| head -c 9000000 > /proc/1/fd/1'`, redeploy, read `logs/`. |
| 6.2, a ceiling that kills | Not run with a server that outgrows its ceiling. | A database at 128 MB loaded with `pgbench -i -s 50` from another container; or nginx with a ceiling just above its idle size and many concurrent requests. Check the page says the kernel killed it (`dmesg` agrees). |

---

## 6. Re-testing what was fixed

Each commit, the fault as it was found, and the check. The checks assume
the VM set up as in [7](#7-the-tools), names in its `/etc/hosts`, and
`scripts/vm_console.py` signed in as the administrator.

| Commit | Fault | Check |
|---|---|---|
| `4a38028` | The domain fell back to the file; the registry named pushes after the name at boot (`localhost/…`); a backup then kept none of the node's own images. `acme.disabled` overruled the console. | Install `--self-signed` with no domain; set the domain from Node → Domain **without restarting**; push with skopeo; `ctr -n wabot images ls -q` names it after the domain; `wabot-deploy backup --out /root/b` lists `image … kept`. Edit `config.toml` (`domain`, `disabled = false`), restart: domain and switch unchanged, `doctor` shows `seed` lines. With the switch off, no form offers `acme` (`scripts/vm_console.py forms /node/domain`), and `submit /node/certificate/source …/source$ renew_with=acme` is refused. |
| `4a38028` | `doctor` before an install stopped at the missing database. | On a fresh VM, before `install`: `doctor` prints `machine` and `runtime`. |
| `8c3de49` | Create a service and add a port at once: `Exited 137`, "cannot delete a running process". | `docker.io/library/httpd:alpine`, then immediately an HTTPS port: the log has `did not stop; killing` and the service ends `Running`; `curl` answers 200. |
| `51b15eb` | A service with no image said "pinned by digest". | Create one with the image empty: the page shows `docker build` / `docker push <domain>/<project>/<service>:latest`. |
| `7ac6eb5` | `*.wabot.test` taken as a port's hostname; the node's domain said "resolves here" of a name elsewhere. | Add `192.168.5.15 *.wabot.test` to the VM's hosts: port form, node domain and organisation domain all refuse it with "a wildcard is a pattern…". `elsewhere.wabot.test` saved as the domain says "resolves to 10.99.99.99". Remove the hosts line after. |
| `3337e54` | A fresh node had WAL archiving off. | Fresh install: `select value from setting where key='wal.archiving'` returns nothing, and Node → Backup has "Keep the write-ahead log" ticked. |
| `094aa26` | `passwd` promised a form to change the password. | `wabot-deploy passwd <user>` output. |
| `f630946` | A restored database's page said nothing about where it came from. | `wabot-deploy restore <db> --to "<moment>" --into <copy>`, deploy the copy: its page has "Restored from" and "Up to". |
| `49397ad` | `orders-ro` failed `verify-full`; a renamed database went back to its old certificate at every deployment; a second copy came up on an address nothing pointed at; an exited container read `Running`. | Database at 128 MB, copies to 2 **in one step**: both copies on `10.42.<n>.25x` on their first deployment; from inside the primary, `psql "…@<db>-ro.<project>…sslmode=verify-full…"` returns `t`. Rename it, redeploy, `verify-full` by the new name still works. Create a `busybox` service: its row says `Not running`. Typing `nginx:alpine` as an image is refused with the qualified form. |
| `9a5ac38` + `39765f4` | 64 MB databases did not initialise reliably. | The new-database form offers 128 MB as its smallest; POSTing `memory=67108864` is refused with the sentence about initdb. |
| `a13d83b` | A removed port's name stayed wanted by the renewal loop; the edges form took any name. | Add a port with a hostname, remove it: `select * from service_edge` has no row for it. POST `…/edges` with `hostname=not-ours.example.com`: refused. |
| `7aa7c17` | `install` restarted an OpenRC node every time. | `install` twice: "the node is already running this binary", same pid. |
| `89f4d42` | `restore-node` took a `log-writer` for the node. | Turn on timestamped logs for a service, redeploy it, stop the node, wait for the process to go: `restore-node` proceeds (with `--over-my-dead-body` on a node with services) instead of naming the log-writer's pid. |
| `761fab1` | A restore brought no image back: blobs over 16 MB in one gRPC message. | Back up a node running a pushed `nginx:alpine` (a 20 MB layer); delete the image and its blob (`ctr -n wabot images rm …`, `ctr -n wabot content rm <digest>`); restore: `images 2 restored`. And a 20 MB monolithic upload: `curl -u <user>:<token> -X POST --data-binary @big "https://<domain>/v2/<project>/big/blobs/uploads/?digest=sha256:…"` answers 201. |
| `9043b27` | "Storage nothing claims · 7" pointed at a page that named none of them. | Restore an older backup over newer services: the card names the containers and the `clean` command. |
| `5e99f49` | Two services without a ceiling under a quota could never be given one. | Quota 64 MB, two limitless services: each takes 32 MB; a third asking 64 MB is refused with the numbers. |

---

## 7. The tools

### 7.1 Setting the VM up the way the walk did

```sh
scripts/vm.sh reset && scripts/vm.sh up       # a clean Alpine node
scripts/vm.sh deploy                          # builds, installs --self-signed, prints the token
scripts/vm_console.py setup <token> <user> '<a phrase of 12+ characters>'
```

Names, inside the VM (a private network's DNS would be this):

```sh
scripts/vm.sh root sh -c 'printf "%s\n" \
  "192.168.5.15 node.wabot.test app.wabot.test shop.wabot.test api.wabot.test db.wabot.test renamed.wabot.test org.wabot.test" \
  "10.99.99.99 elsewhere.wabot.test" >> /etc/hosts'
```

(`192.168.5.15` is the VM's address under Lima's default network; check
with `scripts/vm.sh root ip -4 addr show eth0`.)

What the walk installed in the VM to play the other parties:

```sh
scripts/vm.sh root apk add sqlite skopeo jq postgresql17-client rsync minio minio-client iproute2-ss
# a registry client
skopeo copy --dest-tls-verify=false --dest-creds <token-name>:<token> \
  docker://docker.io/library/nginx:alpine docker://<domain>/<project>/<service>:latest
# an S3 bucket
MINIO_ROOT_USER=minioadmin MINIO_ROOT_PASSWORD=<secret> minio server /srv/minio --address 127.0.0.1:9000 &
mcli alias set local http://127.0.0.1:9000 minioadmin <secret> && mcli mb local/backups
# an SSH destination: this machine, as root
ssh-keygen -t ed25519 -N "" -f /root/.ssh/id_ed25519 && cat /root/.ssh/id_ed25519.pub >> /root/.ssh/authorized_keys
```

### 7.2 Driving the console

`scripts/vm_console.py` posts what a browser with scripting off posts. From
the shell it covers single steps; for a walk, import it:

```python
import sys; sys.path.insert(0, "scripts")
import vm_console as c

s = c.session()                                          # the administrator
r = c.submit(s, "/projects/new", "^/projects$", {"name": "Shop"})
r = c.submit(s, "/projects/shop/services/new", "/services$",
             {"name": "Web", "image": "docker.io/library/nginx:alpine", "env": ""})
print(c.errors(r.text), c.text(r.text)[:500])
print(c.options(c.get(s, "/node/domain").text, "/node/certificate/source$", "renew_with"))

org = c.session("org", host="org.wabot.test")             # an organisation's console
```

Other identities are other cookie jars: `session("dev")` after that person
joined through their invitation link (`/join/<token>`, taken from the page
the invitation is shown on).

### 7.3 Traps

- **Never turn ACME on in the VM, even for a second.** On 2026-10-14 it
  was turned on to test the switch: the renewal loop registered an
  account with Let's Encrypt **production** and ordered `app.wabot.test`
  within a second. It was refused as an invalid identifier (not a failed
  validation, so no lockout), but it reached production. Test the switch
  with it off, and the "on" half only through the page's options.
- **`restore-node` rolls the domain back** to whatever the backup had.
  After the walk the VM answers as `renamed.wabot.test`, and sessions and
  passwords are the backup's too.
- **The node's CA file is root-only**: `sudo curl --cacert
  /var/lib/wabot-deploy/certs/local-ca.crt …`, or copy it out.
- **`rc-service wabot-deploy stop` returns before the process exits**; the
  old `serve` drains for ~20 s. Wait for it before `restore-node`:
  `until ! ps -o args | grep -q "[w]abot-deploy --config"; do sleep 1; done`.
- **A `.is-hidden` element is not on the page.** Reading HTML as text
  without removing them reports badges nobody sees — `vm_console.text`
  removes them.
- **`pgrep -f <pattern>` inside `sh -c '…<pattern>…'` finds itself.**
- The harness in Claude Code refuses `sleep N && …`; poll inside the VM's
  shell instead.
