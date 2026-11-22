# Stability: whether a service stayed within itself

Asked for by Jorge on 2026-11-21: charts of what a service used, with
what happened to it on them, to answer one question — did it stay stable
in memory, CPU, network and disk over time. The page went through three
mock-ups. B3, the one built, is a day-by-day strip, one day up close with
its events on the charts' own clock, and the period's events as a table.

## What is measured

Every node samples the copies **it runs**, once a minute, and folds the
readings into one row per copy per ten minutes (`copy_metric`). Rows are
kept 90 days, events 120. The row is keyed by the container's name, not
by a replica's id, because the name is what both nodes already agree on
and what a log is fetched by.

| Column | Read from | Meaning |
|---|---|---|
| `samples` | the loop | minutes the copy was meant to run |
| `running` | containerd | minutes it was |
| `memory_max` | `memory.current` | the highest minute, page cache included — what the ceiling counts |
| `cpu_avg`, `cpu_max` | `cpu.stat` `usage_usec` | millicores, over the ten minutes and the busiest minute |
| `throttled` | `cpu.stat` `nr_throttled` × the period from `cpu.max` | seconds held at the ceiling |
| `received`, `sent` | `/proc/<pid>/net/dev`, loopback left out | bytes, seen from inside the container |
| `read`, `written` | `io.stat`, every device | bytes |
| `disk` | `volumes::used_by` at the close | the copy's volumes |
| `*_limit` | `memory.max`, `cpu.max`, the service's disk limit | the ceiling then, so a raised one draws as a step |

A counter is a total since the container started, so a reading means
something only against the one before it. One that went backwards is a
container that was recreated, and that interval is dropped rather than
guessed. A process that restarts inside ten minutes writes them again with
what it saw since.

`samples` is what tells two silences apart: a copy that was down has
`running` under `samples`; a node that was not measuring has no row at all.

## What happened

Written where it happens, in the same transaction as the row that changed
when there is one (`service_event`):

| Event | Written by | Where |
|---|---|---|
| deployed | `releases::mark_deployed`, and the deploy job for an image no release names | the owner |
| stopped, started | `services::set_desired_state`, on a change | the owner |
| crashed, OOM kill | `Deployer::watch`, before it starts the copy again | the node running it |
| backed up, backup failed | `backup::once`, per service, from the manifest | the node holding the data |
| standby not following | `ask_replication`, opened and closed on the change | the primary's node |

An OOM kill is told from a crash by exit 137 **and** the guard's
`memory.events` `oom_kill` having risen since the last pass: the copy's own
cgroup goes with the container, and the parent's counter is the only
witness left.

Variables changed come from `config_revision`, with the names that changed
and never their values. What the measurements say is derived when the page
is read and never stored: unmeasured or not running for a quarter of an
hour, the CPU at its ceiling for ten minutes, memory past nine tenths of
its ceiling, a disk past 85 % and 95 % of its own.

## How a day is judged

`domain::policies::stability`, in UTC days. The policy returns a state and
a structured reason, and the page writes the words.

| Row | Attention | Incident |
|---|---|---|
| Availability | a crash or an OOM kill | unmeasured or not running ≥ 15 min |
| Memory | peak ≥ 90 % of its ceiling | an OOM kill |
| CPU | ≥ 10 min at its ceiling | ≥ 120 min |
| Network | peak traffic ≥ 1.8 × the median of the seven days before | — |
| Disk | ≥ 85 %, or full in under 14 days at the last week's rate | ≥ 95 % |
| Backups | a failure, retried or with a good one still in the window | no good one within the cadence and six hours |
| Replicas | a standby not following for less than 15 min | ≥ 15 min |

Disk is shown for a database, a store, or a service with a volume; backups
when there is also a plan; replicas for a Postgres with a standby. A copy
meant to be stopped is not missing, and a day before the first reading has
no data rather than a gap. Without a chosen day, the page opens on the
latest that was not good.

## Copies on other nodes

The owner asks the node running a copy, by the road and credential its log
takes: `POST /api/network/replica-history`, guarded by `placed_by`, a week
at a time from the last ten minutes it already keeps, and a day of events
before that so a span that has since closed is closed here. What it brings
back is kept beside the owner's own rows. A node that cannot be asked is
named above the strip with its reason, and its days go on reading as
unmeasured — which they are, from here. `errand::HISTORY_SINCE` is the
version a node answers from, and **it has to be the version this ships
in**.

## The page

`/projects/<project>/services/<service>/stability?days=30|90&day=YYYY-MM-DD&event=<id>`,
the tab after Overview. Complete without a script: every cell and marker
is a link, the pinned event's card is rendered by the server, and the SVG
is numbers and class names only. The `stability` island adds the tooltips,
the crosshair and the cards on hover, from the numbers each chart was drawn
from. It asks the node for nothing.

## Not yet

- Edge 5xx per service, and a standby's lag in seconds: the mock-up's
  Network verdict and its Retraso chart. Phase 2.
- Memory is read once a minute, so a spike between two readings is not
  drawn: the OOM kill on the VM was 300 MB for a few seconds and the
  chart's peak said 58 MB. The event still makes the day an incident.
  `memory.peak` would see it, and is resettable only from Linux 6.12.
- A copy removed from a service takes its history off the page with it.
- Up to four copies get a colour of their own; a fifth and after are grey.
- Backups are judged against the node's plan, not against which node holds
  the data.
