# The rules

Written down as they are decided, in the order they were decided, with
what each one costs. A rule with no cost written beside it is one
somebody will break for a good reason and not notice they did.

## 0. Six objects, and nothing else

Everything in this tree is one of six things. If a file is none of them,
it is in the wrong place.

| | What it is |
|---|---|
| **Controller** | Receives from outside and responds. Reads a request, calls a use case, renders or redirects. |
| **View** | The visual components — reusable pieces and whole pages. |
| **UseCase** | The actions **and queries** of one kind of user. One file each, named for who does what. |
| **Command** | Shared logic that use cases reuse. |
| **Entity** | The business models, with their rules and their validation. |
| **Repository** | Wraps one entity's data access: the `INSERT`, the `UPDATE`, the `SELECT`. |
| **ValueObject** | A small shared thing that carries its own validation. **Immutable**, and checked once at construction — hold one and it cannot be wrong. |

### The two mistakes this list exists to stop

**A command that only calls somewhere else is not a command.** If the
body lives in another module and the command forwards to it, the command
is a second name for one call. The code goes *in* the command — the
first four written here were wrappers and were deleted, and
`CreateService` was a wrapper too until the body of
`platform::services::create` moved into it.

**A rule about a value is not the entity's either.** `validate_image`
was a free function; making it `Service::validate_image` was still
wrong, because what it validates is *an image reference*, not a service.
It is `values::ImageReference` now — parsed once, immutable after, and
`Service::new` takes the parsed thing rather than a `&str` it has to
re-check.

**An entity is not a bag of fields.** A model that validates nothing and
decides nothing pushes its rules out into free functions, where every
caller has to remember them. `validate_image` was one of those: a rule
about what a `Service` may be, sitting in a module beside the queries.
It is `Service::validate_image` now, and `Service::new` is where a
service's rules are.

### Where each one goes

`src/console` controllers, `src/views`, `src/usecases`, `src/commands`,
`src/entities`, `src/repositories`, `src/values`. `platform` was a name for four of
these at once, which is why the rules ended up wherever the first caller
needed them.

## 1. One use case per file, named for who does what

**The most important one.** A file holds exactly one use case, and its
name says which role performs it and what it does:

```
tenant_user_create_project.rs
tenant_user_update_service_replica.rs
node_user_update_service_replica.rs
cli_user_install_node.rs
remote_node_host_service.rs
```

Shared work lives in **commands** — `UpdateServiceReplica` takes its
parameters and does the job — and a command may be run synchronously or
queued, whichever suits the caller.

### The actors

Everything this node does is done *by somebody*, and there are four:

| Actor | Reaches the node through | What it looks like today |
|---|---|---|
| **Node user** | the machine's console | `console::nodes`, `console::people`, `console::updates` |
| **Tenant user** | an organisation's console | `console::tenant` |
| **CLI user** | a terminal on the machine | `commands::install`, `doctor`, `join`, `passwd`, `backup` |
| **Remote node** | errands and the control-plane API | `network::collect::carry_out`, `network::api` |

**The CLI subcommands are use cases too**, and treating them as a
separate kind of thing was the mistake in the first draft of this rule.
`wabot-deploy join` and the console's join form are one use case with two
actors; `install` is a use case with exactly one. Naming the actor is
what makes that visible instead of arguable.

The fourth is the one that is easy to forget and the one this codebase
has paid for most: a remote node asking for something is not a special
case of a person asking, and every fault in `docs/network.md`'s phase 4-7
list came from code that had not decided which of the two it was serving.


**A fifth: the guest.** Somebody holding an invitation and nothing else —
not yet a user of anything. They are their own actor because what they
may do is *hold a valid token*, and because passing that test is how the
other actors are born: `guest_join_as_node_user` and
`guest_join_as_tenant_user` are two use cases, not one with a parameter,
because the invitation decides which frontier somebody enters and each
door refuses the other kind.

The split is not decoration. The link an invitation carries is built on
its own frontier's address — an organisation's on the organisation's
domain, the machine's on the machine's — and the two consoles are two
routers. `/join` was registered on the node's router only, so every
invitation into an organisation pointed at a door that answered 404, and
1055 tests could not see it because the harness only ever knocked on one
router. It holds both now, and
`an_invitation_link_finds_a_door_in_either_console` is the test that
fails when one of them loses a door.

### Flat, with the actor in the filename

`src/usecases/` holds them all, unnested. The actor is a prefix rather
than a directory, for two reasons:

- **A filename travels.** It is what a stack trace prints, what a grep
  returns, what a review shows. A directory that carries the actor
  leaves the filename saying `create_project.rs`, which is the thing this
  rule exists to stop.
- **Sorting groups them anyway.** `cli_user_*`, `node_user_*`,
  `remote_node_*`, `tenant_user_*` — the listing is the grouping, without
  a path that repeats what the name already says.

### What this fixes

The files it replaces are 9000 and 6000 lines. Nobody reads one whole,
which is how the same rule ends up written twice without anybody
noticing: the placement spread was enforced in two places and neither
could be shown to matter, and a guard that matched one call shape passed
while the other half went untranslated.

It also settles a question that has cost time twice: *whose page is
this*. `console::people` grew a scope selector, a cross-frontier check
and five `administers_scope` calls before anybody read the words on the
page and found they were wrong. With the actor in the filename there is
nowhere for that to hide.

### What it costs

Many small files, and a reader who wants "everything about services" has
to open a directory rather than a file. That is the trade taken
deliberately: a directory listing is a table of contents, and a
9000-line file is not.

### The one thing that is shared: the world a test needs

A use case owns its file, and its tests live in it. What they do **not**
each own is the world they run against — three copies of "an Acme, a
Globex, an administrator, a member, a rival and the machine's own
operator" appeared before the fourth use case was written, and a fixture
copied is a fixture that drifts: the file where somebody adds a second
project is not the file the next test reads.

`usecases::fixtures` is `#[cfg(test)]` and holds it once, returning a
struct with named fields rather than a tuple so a test takes the two
people it needs and ignores the rest. It is not a use case and it breaks
none of this: deleting a use case still deletes everything about it, and
what stays behind is scaffolding nobody ships.

## 2. A command has one body and two doors

`UpdateServiceReplica` is a plain type with an inherent
`run(...) -> Result<T>`. **Synchronous is calling it. Asynchronous is a
`#[command_handler]` whose `handle` calls that same `run`.**

The framework offers only the second half: `wabot::async_jobs::run_command`
enqueues and spawns, and nothing there runs a command and hands back its
outcome. Rather than ask upstream for a synchronous executor — which is
a version, a publish and a wait — the command carries its own body and
the queue is one caller of it.

It is also what the deploy path already does, where
`DeployHandler::handle` calls `Deployer::deploy`. This rule names the
shape rather than inventing it.

### A command earns its file, and one caller does not pay for it

**One synchronous caller means there is no command.** The body goes in
the use case, and the type, the file and the module entry do not exist.
A command is a body two doors share; with one door it is a struct
wrapped round a call, and the indirection is the whole of it.

The first four written here failed this test and were deleted.
`SetPersonRole` was a struct, a lifetime and an `impl` around
`accounts::set_role(database, person, role)` — and the giveaway was that
the node console had been calling `accounts::set_role` directly all
along. **The shared body already existed one layer down.** The command
did not share anything; it re-announced something `platform` was already
holding, in a module whose own doc comment says *one module per
subcommand*.

What makes a command, then: a second door (`#[command_handler]`), or a
second actor's use case reaching the same body. Neither is speculative —
when the node-scope people use cases land and want the same body, that
is the commit that extracts it, and it will be extracting something
whose shape two callers have already shown.

### And a platform function is not a command with one caller

The rule above kills a command at one caller. It does **not** generalise
to every function, and the difference is not arithmetic — it is what the
boundary crosses.

`accounts::set_role` crosses from intent to SQL. It is the only thing in
the tree that knows the `account` table has a `role` column, and it
would earn its file with one caller, because inlining it puts an
`UPDATE` in a use case and the next use case that needs it writes that
`UPDATE` again, differently. `SetPersonRole` crossed nothing: same
language, same argument list, one call, one caller. That is the whole
test — **a layer that changes language keeps its file; a layer that
only changes the spelling does not.**

Which is also how the four were found. Three of them wrapped a function
the node console had been calling *directly* for as long as the console
has existed. The shared body was already one layer down; the command was
a third name for it.

The same reasoning undid a split made in the other direction.
`create_with_token` existed so a command could return `Result<(), _>`
per §4, with the caller choosing the secret so nothing had to come back
out. With the command gone its one caller wanted the token, which is
what `create_into` had always returned — so the two folded back into
one. **A shape adopted to satisfy a layer does not outlive the layer.**

### What it costs

Two ways to reach the same work, and a use case has to choose. The
choice is not free: a synchronous call can tell somebody *why* it was
refused in the same POST, and a queued one can only say it was accepted.
That is the whole reason both doors exist — see the resources form,
which refuses with numbers, against a placement, which cannot.

## 3. Authorisation is per actor, and it is roles

Each kind of user has its own authorisation, and it is small: a handful
of questions asked by name.

```
authz::node_user::may_manage_the_machine(account)
authz::tenant_user::may_deploy(account, project)
authz::remote_node::may_host(authority)
```

**What is shared is the data, not the questions.** `ProjectRole`,
`membership` and the grant tables are one source of truth read by all
four; what differs is which questions an actor is entitled to ask at
all. A node user asks about the machine and a tenant user cannot — not
because the answer is "no" but because the question does not exist in
their module.

### Why per actor rather than one door

Because one door was tried and it drifted. `access::of` is a good
function and it stayed correct; what went wrong was around it — five
`administers_scope` calls in a page that should have asked `is_admin`,
a scope selector on a form that could not honour it, a project list from
the wrong frontier. Every one of those was a *node* use case asking a
*tenant* question, and nothing in the shape said so.

### The use case checks; the command trusts

`authz` answers **what this actor may do** — questions about a role, with
no target. Whether *this particular thing* is theirs is the use case's
job, and it does it by looking the thing up in its own frontier: a
lookup, not a second rule.

A command takes what it is given and does it. It is not a second
authorisation, and giving it one would put the same decision in two
places — which is what these rules exist to stop.

### What it costs, and the rule that pays it

**Four places that can disagree.** That is the real price and it is the
reason the previous draft of this argued the other way. It is paid by
keeping the *data* single: an actor's module may not read a role or a
grant table directly and re-decide it — it composes `platform::access`
and `network::capability`, which stay the one implementation. A
question per actor; an answer computed once.

## 4. A command returns success or a reason, and nothing else

`Result<(), Error>`. One command per file, like a use case —
`update_service_replica.rs`, `drain_node.rs`.

**A command's success is already the description of the new world.** If
it has to hand something back, the caller did not know what it does —
and a caller that does not know what a command does cannot have decided
to run it. That is the whole rule; the return type is the consequence.

### The two that looked like exceptions, and are not

- **The invitation token.** It reads like a return value because the
  function mints it. It is not: the *caller* chooses the secret and
  passes it in, and success means "this token now opens that door". The
  page has the token because it made it, not because it was told.
- **A deployment's address.** After a successful deploy the world
  contains a replica row with an address in it. A caller that wants the
  address reads the world it just asked for — being handed it is a
  shortcut that hides which of the two is the truth, and they have
  disagreed here before.

Both are the same correction: a value coming *out* of a command is
almost always a fact that belongs *in* the world, or a decision that
belonged to the caller all along.

## 5. Controllers are thin; a page is a file

A controller reads the request, calls a use case, and returns what a
**page function** produced. It contains no markup.

One file per page, named the way a use case is:

```
tenant_user_settings_page.rs
node_user_people_page.rs
```

### Why

`services.rs` is nine thousand lines because a handler that renders is a
handler that grows: the query, the decision, the layout and the words
all end up in one function, and the only way to change any of them is to
read all of them. A page that is a file is a page somebody can open.

It is also what makes the actor's words checkable. "On this node" over
an organisation's people was invisible inside a six-hundred-line
handler; in a file called `tenant_user_people_page.rs` it is the second
line.

### What it costs

An indirection on every page, and a page function that needs data has to
be *given* it rather than fetching it — which is the point, and is what
keeps the query out of the markup.

## 6. Zero comments

Not "few". Zero, unless the comment explains **why it was done this
way** — and that is a rare case, not a habit.

The first five rules are what make this possible.
`tenant_user_invite_person.rs` says who and what. `authz` says the
question is about permission. A page file says its contents are words.
Prose repeating any of that is noise between the reader and the part
that matters.

A name that needs a comment is a name to change first.

### What still earns one

Only the reasoning a reader cannot recover from the code: the
alternative that was rejected, or the thing that will look like a
mistake and is not. One sentence. Anything longer is a commit message
or a document.

### The old code

This supersedes `CLAUDE.md`'s density rule for anything written from
here on. Existing files keep what they have until they are moved — a
sweep to delete comments is a diff nobody can review.

## 7. Persistence answers about the data, never about who is asking

`accounts`, `platform` and the rest of the row-writing layer are a
repository: they turn intent into SQL, and they own the tables. A
function there may refuse — **for what the data is**. It may not refuse
for who is holding it.

### The test is not "does it take an actor"

It is: **could two callers legitimately be allowed to disagree?**

- *"Only an administrator can invite another administrator."* No caller
  should ever answer this differently. It is an invariant of the
  `invitation` table, and it belongs beside the `INSERT` — the last gate
  before the row, where no future door can route around it.
- *"Only an administrator of this organisation may run this use case."*
  This is per-door by construction: the node's operator has their own
  page, and the answer there is different. It belongs in the use case.

So `invitations::create_into` taking `&Account` is not a violation. It
reads the creator's frontier to know *which one is theirs* — that is
data. Contrast a use case, which reads the same account to decide
whether to proceed at all.

### What the tree actually looked like

Measured before writing this rule, across ~13,000 lines:

| | |
|---|---|
| pure SQL, a repository already | 81 |
| refuse on an invariant, no actor involved | 21 |
| read and take an `&Account` — `access::*`, whose whole job is that question | 3 |
| write and branch on the asker's role | **0** |

The discipline was already being followed. What was missing was the
sentence, and something that keeps it true.

### The guard

`authz::tests::nothing_that_writes_a_row_asks_who_is_asking` reads
`src/accounts` and `src/platform` off the disk and fails if any of them
imports `crate::authz`. It globs the directories rather than listing
files, because *a guard that names its inputs is right until somebody
adds an input* — which has cost this project four times.

It is written as "never imports `authz`" rather than "never calls
`is_admin`" on purpose: the first is a layering fact with no exceptions,
and the second would need an allow-list, which is the same failure in a
new coat.

### What it costs

An invariant and an authorisation can look identical at the call site,
and telling them apart takes the question above rather than the shape of
the signature. When it is genuinely ambiguous, put it beside the write:
a rule enforced too often is a page that refuses something it could have
allowed, and a rule enforced too rarely is a row that should not exist.

## 8. A button says it is working, and the page does not move

Pressing a button runs its action over `fetch`. The button disables
itself for the round trip, says so, and shows what happened when it
knows — a check, a cross — and none of it costs a page load, a scroll or
a lost place in a list.

### Where it lives, and why not here

In the framework. Boosted navigation already covered links, with
`apply`, `swapContent`, `ensureStyles` and the scroll decision all
written; a second copy of that in `assets/console.js` would have been
the worse of two. `submit` is intercepted beside `click`, in the same
file, and every wabot app inherits it.

### Four things that are invisible when wrong

- **It is enhancement, not a requirement.** With scripting off the form
  posts exactly as before and the server sees the same request. This is
  the console somebody opens when the node is unhealthy — see
  `CLAUDE.md`, "The console works without JavaScript".
- **Urlencoded, not multipart.** `FormData` over `fetch` sends
  multipart, and a server that parses one does not parse the other.
  Getting this wrong breaks every POST at once, silently. A multipart
  form is left to the browser, where upload progress belongs.
- **No scroll on the result.** `apply(data, false)`. Jumping to the top
  after every action is the whole complaint this answers.
- **A refusal arrives as a redirect, not as a status.** This console
  answers 303 whichever way an action went and says which by putting a
  message in the query. Reading the status alone puts a green check on
  every rejection, so the runtime reads the landing URL's `error`
  parameter instead.

### The opt-out, and the three that use it

`data-wabot-reload` on the form, the same spelling a link already uses.
`/setup`, `/sign-in` and `/join` carry it: entering the console replaces
the whole document — the shell, the navigation and everything that
depends on a session appear for the first time — and swapping a fragment
into the page you are leaving is not that.
`the_forms_that_let_somebody_in_are_not_boosted` fails when one loses it.

### What it costs

Nothing in either workspace executes `client.js`, so the behaviour is
not covered by any suite. What the tests reach is the server contract it
rests on — a redirect followed with the nav header has to answer with a
fragment — and four assertions read off the runtime's source. **The rest
is verified in a browser by somebody**, which is this project's standing
rule for anything visual.

