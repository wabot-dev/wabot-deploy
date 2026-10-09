# The rules

Written down as they are decided, in the order they were decided, with
what each one costs. A rule with no cost written beside it is one
somebody will break for a good reason and not notice they did.

## 0. The objects, and nothing else

Everything in this tree is one of these. If a file is none of them, it
is in the wrong place.

| | What it is |
|---|---|
| **Controller** | Receives from outside and responds. Reads a request, calls a use case, renders or redirects. |
| **View** | The visual components — reusable pieces and whole pages. |
| **UseCase** | The actions **and queries** of one kind of user. One file each, named for who does what. |
| **Command** | Shared logic that use cases reuse. |
| **Entity** | The business models, with their rules and their validation. |
| **Repository** | Wraps one entity's data access: the `INSERT`, the `UPDATE`, the `SELECT`. |
| **ValueObject** | A small shared thing that carries its own validation. **Immutable**, and checked once at construction — hold one and it cannot be wrong. |
| **Policy** | A pure rule that belongs to no single entity: a table, a ladder, an admission sum. No storage, no actor, no identity. |
| **Request** | What one use case needs to be asked. A controller builds it from a form; the use case reads nothing else. |
| **Response** | What a use case hands back — never an entity. |
| **Dto** | A shape more than one request or response carries. |

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

A feature's controllers, pages, components, use cases, commands,
requests and responses go in its own folder under `src/features`, each
named for what it is — §11. What every feature reads stays a layer of
its own: `src/entities`, `src/repositories`, `src/values`,
`src/policies`, and the console's shell in `src/console` and
`src/views`. `src/dtos` holds the shapes more than one feature
carries. The terminal's verbs are `src/cli`, one file each, and the
machinery a backup is taken and restored with is `src/backup`, which the
CLI and the console's backups feature both call.

`platform` was a name for five of these at once, which is why the rules
ended up wherever the first caller needed them. It is being taken apart.

A **policy** is the one that is easy to mistake for a repository because
it lives near the data. `presets` is a ladder of sizes; `postgres` is the
arithmetic a memory ceiling implies for the engine; `wal` is how long the
log is kept. None of them stores anything, none of them is about one
entity, and all of them are pure — which is what makes them the easiest
things in this tree to test.

**And one that reads a row is not a policy, whatever it decides.** Four
lived here that did: `access`, which reads and writes the membership
table, and `project_domain`, which reads a tenant, are repositories;
`quotas`, which sums what an organisation has taken and asks
`deploy::fits`, and `upstreams`, which picks a name's targets from the
network's nodes, are the deploy path's. The test is the definition above
— no storage — and the name of a module is not.

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
| **CLI user** | a terminal on the machine | `cli::install`, `doctor`, `join`, `passwd`, `backup` |
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

### In its feature's folder, with the actor in the filename

A use case lives beside the command, the request, the controller and
the page it is made of —
`src/features/databases/tenant_user_restore_database.rs` — and the actor
stays a prefix rather than a directory:

- **A filename travels.** It is what a stack trace prints, what a grep
  returns, what a review shows. A directory that carries the actor
  leaves the filename saying `create_project.rs`, which is the thing this
  rule exists to stop. A directory that carries the *feature* takes
  nothing away: the file still says who and what.

They were all in `src/usecases/` first, flat, on the argument that
sorting groups them anyway. It did — by actor, which is the grouping
nobody arrives with. Changing how a database is restored meant knowing
which eight directories to open; §11 is what replaced it.

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

`features::fixtures` is `#[cfg(test)]` and holds it once, returning a
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
authz::node_user::manages_the_machine(account)
authz::node_user::belongs_here(account)
authz::tenant_user::administers(account)
```

**Questions about a role, with no target, and so no database.** The
first draft of `tenant_user` also carried `may_deploy(account, project)`,
reading the membership table to answer it. It was never called: every
use case that needed it looked the project up in its own frontier and
asked the `Access` it got back, which is the next section's rule. It is
gone, and `authz` is pure.

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
grant table directly and re-decide it — it composes `repositories::access`
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

### A button never changes size

Not while it works, not when it reports, not for an animation. A control
that grows under the cursor moves whatever is beside it, and on a row of
them it moves the one somebody was about to press.

So the mark does not sit **after** the label, it sits **over** it: the
label goes transparent and the check is drawn across the whole button.
Appending it was the first attempt and it was wrong twice over — `.btn`
is `inline-flex` with a `gap`, so a `::after` becomes a flex item, which
widened the button *and* pushed the label off centre.

`nothing_about_a_button_animates_a_dimension` reads both stylesheets and
refuses any rule about a button that transitions or animates `width`,
`height`, `padding`, `margin`, `font-size`, `transform`, `scale` or
`all`. It is a fact about the declaration, not a list of today's rules,
so a variant added tomorrow is covered without touching it.

### The mark is the button's own text colour

It was a fixed green and red — `--c-success-fg` and `--c-danger-fg` —
which read on a pale card and vanish on the black primary button, where
almost every confirmed action in this console happens. Reported by Jorge.

`currentColor`, not a rule per variant: primary, danger and brand are
dark with light text, secondary and ghost are light with dark text, and a
list of the three would be the fault this project has paid for five times
— right until somebody adds a sixth. This is legible on any button by
construction.

Colour is not the signal and does not need to be: **the glyph is.** A
tick and a cross tell them apart, which is the rule `service_state.rs`
already writes down — a colour alone says nothing to somebody who cannot
tell two of them apart.

Hiding the label then needs a property that takes the **glyphs** and
nothing else, and the two obvious ones each take something the mark
needs. Both were shipped, and both left nothing on the button:

- `visibility: hidden` hides the element, background included. The dark
  button went white for the round trip and the check — drawn in its light
  text colour — vanished on it.
- `color: transparent` keeps the background and blanks `currentColor`
  with it, so the mark is drawn in the same nothing as the label.

`-webkit-text-fill-color` fills the glyphs and leaves `color` alone,
which is the pair of things wanted. Prefixed, and a de facto standard.

`the_mark_on_a_button_is_the_colour_of_its_own_text` asserts the mark
takes no fixed colour **and** that the label is hidden this way. Its
first version named `color: transparent` as the thing to refuse and let
`visibility` straight through — a guard written against the last mistake
catches the last mistake. It recognises the rule by the job it does now.

### An icon-only button gets no mark

It has no label for one to replace, so a check lands beside the glyph and
the control reads as two things — a stop square with a tick next to it,
reported by Jorge from the page. What tells you it worked is the thing it
changed: the badge goes to Running, the row goes away. It still dims
while it works, which is the half of the feedback that costs nothing.

**And it is spelled one way.** It was spelled two: the play and stop
controls said `.icon` while every other icon-only button said
`.btn-icon`, so the rule that withholds the mark was looking at the other
name. That is the same fault as a guard naming its inputs, in CSS.
`an_icon_only_button_is_spelled_one_way` reads the view directories.

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



## A secret shown once gets a page of its own

Not a card bolted to the top of whatever minted it. The invite page came
back with the token above its own form; an organisation came back with
the link above its quota — so the one thing somebody has to act on *now*
arrived surrounded by the controls they had just finished using, and the
only way to clear it is a reload, which is what spends the nonce. Jorge:
*"es muy extraña la experiencia"*.

One page, one thing on it, and the breadcrumb's arrow is the way back —
the page draws no button for it, per
`no_page_offers_a_way_back_the_breadcrumb_already_is`. Four routes and
one view: what the page *says* depends on which secret it is, and the
route is what knows, so the kind comes from the path and the value comes
from the nonce.

**The arrow and a spent nonce go to the same place.** A refresh landing
somewhere the arrow does not point is two answers to "where was I".

**The store holds what goes on the clipboard**, not the raw secret. A
join token becomes `wabot-deploy join …` and a push token becomes the
whole `docker login …` before either is stashed — the page chooses the
words around a value, never the value.

### And a secret never travels in the query string

`ConsoleState::reveals` was written because a push token used to, and it
was written as if that were the only one. **Three invitation links were
still doing it** — the node's people page, an organisation's, a
tenant's. A query parameter is read by the address bar, the history and
every refresh, and an invitation link is a secret: whoever holds it
becomes an account.

`a_redirect_carries_a_nonce_and_never_a_secret` reads the controllers
for the spellings a secret has travelled under. A fact about the
redirect, not a list of the handlers that mint something.

## 10. One controller per file, named for what it is

`<Subject><Kind>Controller`, in `<subject>_<kind>_controller.rs`.
`NodeSettingsUiController` in `node_settings_ui_controller.rs`;
`ServiceRestController` in `service_rest_controller.rs`.

There were twenty-two of them in thirteen files, most two to a file
under names that said neither what they were nor which kind:
`NodePages` and `NodeApi`, both in `nodes.rs`. Two controllers in one
file is two things a stack trace names identically, and a name carrying
neither leaves the reader to open the file to find out.

### Splitting is what shows you what was shared

Every single thing two controllers had in common turned out to be in the
wrong place, and none of it was visible while they sat in one file:

- `replica_cell`, `edge_cell` and three cell types were imported **out**
  of `console/projects.rs` by three view components — the dependency
  pointing the wrong way. They are `views::components::project_cells`.
- `Live` and the builders that fill it are read by both halves of that
  file, which is exactly why they belong to neither: `console::live`, so
  the first paint and the stream cannot disagree.
- `source_from` reads a form, so it joins the other form readers in
  `console::auth`. `SOURCES` is what a form offers, so it is the view's.
  `store_source` writes and three controllers reuse it — a command.
- `project_slug` parses a path both halves parse.

### What it costs

A helper used by two controllers has nowhere to hide. That is the point,
and it is also the work: each one has to be given a home that says what
it is, and "it was next to its caller" stops being an answer.

### The guard

`a_controller_is_alone_in_a_file_named_for_it` walks the whole tree —
not just the console, because the registry, the network's api and the
health check are controllers too, each of which was in a file named for
its module. It checks all three halves of the rule: one per file, a name
ending `UiController` or `RestController`, and a file name that is that
name in snake case.

Four other guards named console files by name and went red the moment
these were split. Three now read directories; the fourth, the autofill
scan, listed eight files — so **every field that moved into
`src/views/pages` when the controllers stopped drawing had never been
checked at all.**

## 9. A use case takes a request and hands back a response

The two ends of a use case are named shapes: a `_request.rs` and a
`_response.rs` in its feature's folder, each holding a type named for
its file — `CreateDatabaseRequest` in `create_database_request.rs`
(§11). `src/dtos` holds only the shapes more than one feature
carries. Never a form, never an argument list that grew, never an
entity coming back out.

```
form ──▶ Controller ──▶ Request ──▶ UseCase ──▶ Response ──▶ Controller ──▶ View
```

**The controller builds the request, and it arrives validated.** By the
time `run` is called the shape is right: a name that is a name, a
project that was named at all. What the request cannot answer — whether
the project exists, whether this actor may deploy in it, whether the
node owns services — stays inside, because none of it can be asked of a
form.

### Not every use case has one

A request earns its place when the ask has more than one part, or when
it comes off a form: those are the two cases where a shape goes wrong
silently. A use case given one identifier the router already parsed
takes it as an argument. A struct around a single `&str` is a name to
keep agreeing with and buys no check the type did not already make.

### A rule with a better sentence stays where the sentence is

`Validate` answers *a field arrived, and it is the right length*. It
cannot say **no release chosen**, or **that project belongs to an
organisation — invite them from its own page**. So the derive covers
shape and the use case keeps every sentence somebody can act on. A
constraint written in both places is a constraint that will disagree
with itself.

That is also why several requests carry **text where a number is
meant** — `keep_days`, `memory`, `cpu`, `disk`. Typed here, serde
refuses first and says *invalid digit*; left as text, `presets::parse_size`
says which value it could not read and what the grammar is. Two fields
go further and refuse nothing at all: an unreadable `hour` or `weekday`
takes the default and is clamped, because a backup that runs at the
wrong hour beats a form that will not save.

### The form's own spelling stops at the controller

An unticked checkbox sends **nothing at all**, so presence is the
answer — and `accept-host`, `require-edge`, `own` are the browser's
words for it. A use case that knew them would be a use case only a
browser can call, leaving the CLI and an errand no door. So the
controller translates: `capabilities_ticked(&form, "require-")` returns
`Vec<Capability>`, and `Capability` serialises as the lowercase name it
already had, held to it by
`a_capability_serialises_as_the_name_it_is_known_by` over `ALL`.

### What was actually wrong

Six `Asked` structs and five outcomes were declared beside the `run`
that used them, three of them inside `commands/` with a doc comment that
said *"what a form asks for"*. One use case took a `HashMap` **and a
closure over it**. Two console handlers each built their own shape for
the same ask. None of it was caught by a test, because every one of them
compiled.

### The guard

Four, and all four read directories rather than a list — *a guard
that names its inputs is right until somebody adds an input*, which has
cost this project five times:

- `a_use_case_declares_no_shape_of_its_own`
- `no_use_case_reads_a_form` — `HashMap`, `read_form`, `axum`,
  `form_urlencoded`
- `every_file_in_a_feature_is_compiled`, which replaced
  `every_dto_written_is_a_dto_the_tree_knows_about` when the last DTO
  left `src/dtos/requests`
- `a_request_or_a_response_is_named_for_its_file`

The first two walk all of `src` and know a use case by its actor prefix,
not by its directory: they read `src/usecases` until the first feature
moved, and would have stopped reading every use case that left it. They
carry a floor (`read > 50`), because a scan pointed at the wrong place
finds nothing wrong and passes.

### Mapping

`wabot::mapper::map` exists in the framework: it serialises,
deserialises and **validates**, agreeing on field *names* rather than
types, with no "just convert it" variant on purpose. It round-trips
through `serde_json::Value` — right for a request off a form, wrong for
a hot loop. Where a response is built from an entity, write the fields
out: `EnvAppliedResponse { service_id: applied.service.id, .. }` is where the
entity stops, and the compiler is what says which field is missing.

### What it costs

A shape to name, and a file for it. And two structs where one nearly
worked: `set_service_env_command::Applied` carries the `Service`
because the command's callers are in the domain, and the use case is
what turns it into `EnvAppliedResponse`. Where a command's return has no entity
in it — `NameDatabase` — the command returns the response itself and the
duplicate does not exist.

## 11. A feature is a folder

Everything one feature is made of sits in one directory under
`src/features`, and the filename says what each file is:

| The name | What it is |
|---|---|
| `node_user_…`, `tenant_user_…`, `guest_…`, `cli_user_…`, `remote_node_…` | a use case (§1) |
| `…_command.rs` | a command (§2) |
| `…_request.rs`, `…_response.rs` | what a use case is asked and hands back (§9) |
| `…_rest_controller.rs`, `…_ui_controller.rs` | a controller (§10) |
| `…_page.rs` | a page (§5) |
| `…_component.rs` | a piece a page is drawn from, kept in a file of its own |
| `texts.rs` | the feature's words, in both languages |

```
src/features/databases/
  create_database_command.rs        node_user_create_database.rs
  create_database_request.rs        node_user_restore_database.rs
  database_cards_component.rs       restore_database_command.rs
  database_rest_controller.rs       tenant_user_create_database.rs
  new_database_page.rs              texts.rs   …
```

### Why

Measured before the first move: the databases feature was twenty-two
files in eight directories — `usecases`, `commands`, `dtos/requests`,
`dtos/responses`, `console`, `views/pages`, `views/components`,
`views/texts`. Every one of those directories was right about what its
files were, and none of them said which feature a file served. A layer
directory answers *what kind of thing is this*; the question somebody
arrives with is *where is the thing I am changing*.

The layers did not go anywhere. A file is still exactly one kind — the
controller does not draw, the use case does not read a form — and that
is carried by its name now instead of its directory. Sorted, a folder
reads as the feature's table of contents: the verbs group
(`create_database_command`, `create_database_request`), and the actors
group (`node_user_…`, `tenant_user_…`).

### The kind is in the name because the guards read names

A directory holding every kind cannot tell a guard what a file is.
Thirteen guards knew what they were reading by where it lived —
`src/usecases` meant a use case, `src/console` a controller,
`src/views/pages` something drawn — and every one of them would have
stopped reading a database file the moment it moved, and stayed green.
`crate::tree` walks directories and reads the kind off the name, and
each of those guards reads `src/features` as well. Before the first move
was called done, eleven of them were seen red on a fault planted inside
a feature folder; of the other two, one shares its scan with a guard
that was, and one asserts that a sentence exists somewhere, which
reading more files cannot break.

Two guards hold the folder itself to it:
`every_file_in_a_feature_says_what_it_is`, because a file with no kind
in its name is one every guard reads as nothing, and
`every_file_in_a_feature_is_compiled`, because a file its `mod.rs` does
not declare compiles nowhere and drifts with the suite green.

### A request is named for its file

`create_database_request.rs` holds `CreateDatabaseRequest`, the way
§10 names a controller for its file; a response is the same. The
command beside it is still `CreateDatabase`, because §2 names a command
for the job — and that is exactly why the request needed the suffix:
both were `CreateDatabase`, and a use case importing the two could only
be read by their paths. `a_request_or_a_response_is_named_for_its_file`.

### What stays outside

What every feature reads: entities, values, policies, repositories, the
runtime, the edge. And the console's shell — the router, the session,
the layout, the language, and the pieces that show nothing of any
feature's (`mark`, the logo a page without a header opens with;
`action_button`) — stays in `src/console` and `src/views`.

**Any other component goes with the feature whose thing it shows, and
the pages that draw it say which better than its name does.** `service_table` and `people_card`
are drawn only by project pages and show a project, so they are the
projects feature's; `project_cells` is drawn only by service pages and
shows a replica, so it is not. `attention_card` is drawn only by the
projects page too, but because that page is the console's home: what it
shows is the node's, so it stays out until attention has a folder of
its own.

**A component two features draw stays out of both.** `node_capabilities`
and `node_settings_cards` are drawn by the network's pages and by this
machine's settings, so they stay in `src/views/components` — and the
day one feature stops drawing one, it moves to the other.

**A use case is placed by the same question.** Joining by invitation is
a guest's — `guest_join_as_node_user`, `guest_join_as_tenant_user` — but
the invitation is people's: it is made, withdrawn and spent through the
people controllers, so the two joins and the page they post from live
in `people`. Signing in and claiming the node are not about anybody's
invitation, and stay out of it.

A feature's words are its own `texts.rs`, and `views::texts::TABLES`
lists each one by its path. `the_lookup_searches_every_table_there_is`
finds them by walking `src/features`, so a feature whose table is not
listed fails by name.

### Where it stands

Every feature has moved — `access`, `backups`, `databases`, `network`,
`node`, `organisations`, `people`, `projects`, `services` and
`updates`. What is left in the layer directories is shared: the
console's shell, the components and words more than one feature uses,
the commands more than one feature calls, and the CLI's verbs.

### What it costs

- **Longer paths.**
  `crate::features::databases::create_database_command::CreateDatabase`.
- **A page that draws another feature's piece imports across folders.**
  The service pages draw the database cards; the project page draws
  `service_state`, which will be the services feature's.
- **A door can sit in the wrong feature, and the folder makes it
  visible.** Restoring a database was answered by the service
  controller, calling the databases feature's use cases from outside it,
  because the form is on a database's service page. It moved to the
  database controller on the same route, before services moved.
- **Code beneath a feature can reach up into it.** `deploy` calls
  `qualified_name` out of `create_database_command.rs`. That dependency
  pointed the wrong way before the move; the move is what made it
  readable.
- **The shell keeps directories of its own.** `src/console` and
  `src/views` hold what no single feature owns, so a guard that reads
  everything that draws lists them beside `src/features`.
