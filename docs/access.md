# Access: passkeys, trusted devices, and no password to remember

Asked for by Jorge on 2026-11-21: nobody should have to remember a
password, and signing in should be safer for it, not only easier. Today
the setup page, the invitation and the sign-in page each ask for a
password, and `wabot-deploy passwd` is the way back for an operator who
forgot theirs.

Agreed with him the same day:

- **A passkey is the way in.** Created when somebody claims the node or
  accepts an invitation, with nothing to type but a username. The site
  asks the device to verify its user. The device decides how: a
  fingerprint, a face or a PIN. A passkey on a phone signs in on a laptop
  without a sensor, over a QR code.
- **A device you trust needs only the passkey. A new one needs a second
  factor**, and is trusted afterwards only if you say so.
- **The second factor is a device you already trust.** Usually the phone,
  with the console open in its browser. No app, no push service, no
  secret to back up. TOTP was considered and not taken: it would put a
  shared secret on the node, authenticator apps sync to the same cloud
  accounts passkeys do, and it cannot tell you somebody else is trying.
- **No password behind it.** When a browser cannot make a passkey, the
  way in is a one-time link from somebody above you.
- **A password that exists today keeps working until its owner adds a
  passkey**, and is deleted then. Nothing creates a new one.
- **Verifying a passkey is the framework's**, as an addon. Storing them,
  the devices, the links and the pages are this repository's.
- **A tenant's service is named under its organisation's domain, never
  under the node's** (Jorge's own addition). See "Same site".

## What a passkey is bound to

**The host it was made on.** WebAuthn calls it the relying party id, and
it is a domain:

- **Not an IP address.** Browsers refuse one, so a node reached by its
  address, as one installed without a domain is, cannot make passkeys.
- **Not behind a certificate error.** Chrome turns WebAuthn off on such a
  page. A `--self-signed` node works only for a browser that trusts its
  CA; `localhost` is a valid id, so the VM works once
  `.dev/data/certs/local-ca.crt` or the VM's CA is trusted.
- **Not across a rename.** A passkey made for one name answers for that
  name only, so renaming the node, or an organisation, leaves every
  passkey of that console useless. The domain form says how many people
  that is before it saves.

Accounts already belong to one host. A username belongs to a scope
(migration `0057`), the scope is read off the `Host`, and the session
cookie is host-only. So the node's people have passkeys for the node's
domain and an organisation's people for the organisation's.

**The server compares the origin exactly**, not only the hash of the
relying party id. A page on any subdomain of the console's host may ask
for the console's passkeys, and what makes the answer useless to it is
that its own origin is signed into it.

## A device is a browser, not a passkey

Apple always syncs passkeys, and Google almost always does. A new Mac
signed into the same iCloud account has the passkey before this node has
heard of the Mac. So the passkey cannot say which device is asking.
Possessing it means controlling that cloud account, and that is the gap a
trusted device closes.

A device is a token in a cookie, `__Host-wabot_device`. It is HttpOnly
and long-lived, and it is stored hashed as sessions are. Clearing
cookies, a private window or a new browser profile is a new device.

| Column | |
|---|---|
| account | whose trust it is |
| name | `Safari on macOS`, from the request |
| created, last used, last address | what the list shows |
| credential | the passkey it was trusted with |

Trust lasts until it is revoked, or until 90 days pass without use.
Revoking a device ends its sessions.

## Signing in

**On a trusted device:** "Sign in with a passkey". The credential is
discoverable, so nobody types a username, and the username field offers
the passkey as autofill too.

**On a new device:**

1. The passkey passes. The node opens a request for this browser: at most
   one pending per account, five minutes, five attempts, bound to the
   browser that made it.
2. Every trusted device of the account shows it on its next page, and
   live while a page is open: "Firefox on Windows · 190.x.x.x · a minute
   ago — It was me / It was not me".
3. "It was me" asks for the passkey on that device, then shows six
   digits. They are typed on the new device, so nobody approves by
   accident, and the new device needs nothing but a form.
4. "It was not me" refuses the request and says which passkey was used.
   It offers to remove that passkey, because somebody else has it.
5. The new device asks "Trust this device?". If yes, it gets the device
   token. If no, the session ends with the browser and the next sign-in
   asks again.

**The first device is trusted on its own**: at setup the setup token is
the second factor, and on joining the invitation is.

**A device with no passkey yet**, the phone usually: "Add a device" on a
trusted one asks for the passkey and shows a QR code with a link. The
link works once, for five minutes. The phone opens it, makes its own
passkey, and is trusted. Setup suggests it at once, because two trusted
devices mean losing one does not need a link.

## Links

One mechanism, the invitation's: a token stored hashed, spent on use, and
short-lived. What differs is who issues it.

| Issued by | For | Lasts |
|---|---|---|
| a trusted device of yours | adding a device | 5 minutes |
| an administrator, from People | somebody they could remove | 15 minutes |
| the node's operator | an organisation's administrator | 15 minutes |
| root, from the terminal | anybody on the node | 15 minutes |

**Never upward.** An organisation's administrator never issues one for
the node's people, and only for those they could remove. A link
signs its holder in and counts as the second factor. It offers to make a
passkey and trust the browser. Every link used is an event its owner
sees, with who issued it.

`passwd` prints a link instead of a password, and its name changes with
it.

A link is what covers every case the passkey cannot: scripting off, an
address instead of a name, a lost device, a renamed domain.

## Asked again before anything that makes a way in

A stolen session cookie must not become permanent access. Adding a
passkey or approving its own device would do exactly that. These ask for
a passkey verified in the last five minutes:

- "It was me", and "Add a device";
- adding or removing a passkey;
- issuing a link, or an invitation;
- minting a push token or a bucket key.

On a trusted device that is one touch. Without scripting it cannot be
done. A session a link made counts as verified for its first five
minutes, which is the way through without it.

## Sessions

The sign-in is protected. A stolen cookie is not, because malware that
steals sessions takes every cookie. So:

- a session belongs to the device it was made on, and dies with it;
- on a device not trusted, the cookie ends with the browser. The row also
  ends twelve hours after it was made, because a browser that restores
  its tabs restores session cookies too;
- on a trusted device, seven days, as today.

The session cookie becomes `__Host-wabot_session`, which signs everybody
out once.

## What happened

No email and no push, so an event has to be where the person looks:

- **Your account › Activity:** device trusted, passkey added or removed,
  link used and who issued it, a request refused.
- **At sign-in**, a note for anything since the last visit.

Your account is the first page somebody has for themselves. It belongs to
the session, so it sits in the header beside the language and the theme,
with three tabs: Passkeys, Devices and Activity. An administrator can sign a
person out everywhere and withdraw every device of theirs, from People.

## Same site

Browsers decide `SameSite` by the **registrable domain**, not by the
host. `app.shop.node-0.network.nubebasura.lat` and
`node-0.network.nubebasura.lat` are one site, and so is
`acme.nubebasura.lat`. A service's name used to default to
`<service>.<project>.<the node's domain>`. That puts an organisation's
own pages, or a public bucket's objects, on the same site as the node's
console. Such a page, opened by an administrator, can:

- **post to the console with the administrator's cookie.** `SameSite=Lax`
  does not stop a same-site POST, so it can change a role, an allowlist,
  or delete a project;
- **plant cookies for the parent domain**, which the console's host
  receives;
- **ask for the console's passkeys**, harmless only because the origin is
  compared exactly.

That is read off the code, not yet reproduced. Three answers, all of them
phase 0:

- **A request carrying a session cookie is refused unless its `Origin`
  names the host it was sent to.** `Sec-Fetch-Site: same-origin` stands
  in for a client that sends no `Origin`. Forms send it with scripting off
  too, so the console keeps working without JavaScript. The registry is
  untouched: it authenticates with push tokens, not cookies. Generic, so
  it is a middleware in the framework.
- **`__Host-` cookies.** A subdomain cannot set one.
- **A tenant's names stay off the node's domain.** `project_domain::
  domain_for` already proposes the organisation's domain for its projects.
  It falls back to the node's when the organisation has none, and it is
  only a proposal: `add_service_port_command` accepts any name that
  resolves here. So:
  - an organisation's project with no organisation domain is offered no
    name, rather than the node's;
  - a hostname under the node's domain, or under another organisation's,
    is refused for a tenant's project. A name of their own, like
    `shop.example.com`, is not;
  - names saved before keep working. The rule is asked when a name is
    saved.

The third does not replace the first. An organisation whose domain
shares the node's registrable domain is still the same site, and an
organisation's own services are the same site as its own console.

## Restore and rollback

- **`restore-node` from an older backup brings back what was revoked
  after it**: devices and sessions. It ends every session, and its output
  says that revocations since the backup's date were undone.
- **Going back to `wabot-deploy.previous` after passwords were deleted**
  leaves that binary no way to sign anybody in. Its own `passwd` writes
  one. The release that deletes passwords says so in its notes.

## Not solved here

- **Every node is its own console.** Accounts are per node by decision
  (`docs/network.md`, "no delegated account"), so an operator with three
  nodes has three passkeys and trusts each device three times. One
  identity across nodes would be its own decision.
- **Push.** Without it, the phone sees a request only once it opens the
  console. Web Push (VAPID) needs nothing `ring` does not already have,
  and the node can reach Apple's and Google's push services. Later.
- **Devices with no authenticator.** A Linux desktop or an old machine
  signs in with the phone over QR, or with a security key. To be tried,
  not assumed.

## Verifying it

`scripts/vm_console.py` runs with scripting off, so it can walk links,
codes, the origin check and the hostname rule, but never a passkey
ceremony.

- The addon is tested against real responses recorded from browsers, as
  vectors.
- On the VM, with its CA trusted.
- Jorge looks: Safari and iOS, Android, Windows Hello, Firefox, and a
  desktop over QR.

No `webauthn-rs`: it needs OpenSSL, and the binary has none. It is
rustls and `ring`, cross-built to musl with zig. Without attestation,
since which make of device it is does not matter, verifying is reading
CBOR and a COSE key, a signature `ring` checks (ES256, EdDSA, RS256), and
the challenge, origin, relying party hash, flags and counter. The page
renders the challenge. An island named `passkey` calls
`navigator.credentials`, fills hidden fields and submits the form. It
fetches nothing the page did not have.

## Phases

0. **Same site.** The origin check upstream, `__Host-` cookies, and the
   tenant's names. Independent of the rest, and it protects the console
   as it is.
1. **Passkeys.** The addon, setup, invitation, sign-in, Your account ›
   Passkeys, and the existing password deleted with the first passkey.
2. **Trusted devices**, asking again, sessions bound to devices, and
   Activity.
3. **Approval** from a trusted device, and "Add a device".
4. **Links**: issued from People, by the node's operator, and by root
   through what `passwd` becomes.

## Where it stands

All five phases are written, on the branch `access-passkeys`
(2026-11-21), with the gate green against `wabot-feature-rest-controller`
0.2.5 and `wabot-addon-auth-passkey` 0.2.0 as published (2026-11-22), and
released as the v0.46.0 beta the same day. Jorge ran it on real nodes: the
ceremonies held, and the pages were redone (below).

- Five tests walk the ceremonies over HTTP with the addon's software
  authenticator (`features::account::account_rest_controller`): setup
  trusts its browser and the passkey alone signs in there; a passkey on a
  new browser waits for "It was me" and the code; "It was not me" closes
  the request and names the passkey; a session older than five minutes is
  asked again before "Add a device", and the link trusts the phone; the
  first passkey deletes a password and trusts the browser.
- Asked again before: "It was me", "Add a device", adding or removing a
  passkey, a sign-in link, an invitation (People and an organisation's
  administrator), a push token and a bucket key.
- People has "Sign-in link" and "Sign out everywhere" on every row an
  administrator could remove; the node's operator has both for an
  organisation's administrators, on its Owner tab.
- `wabot-deploy sign-in-link <username>` prints one link per account of
  that name, each on its own console's address; `passwd` is an alias.
- `restore-node` ends every session and says that what was revoked since
  the backup is back.
- A domain form says when passkeys answer for the current name.

### The second look (2026-11-22)

v0.46.0 ran on real nodes and the mechanism held; the pages did not. What
changed, from FIDO's, Google's and Chrome's guidance on passkey UX:

- **Signing in** is one column with no cards, saying which console it is.
  The username field carries `autocomplete="username webauthn"` and a
  conditional request waits on it, so the browser offers this host's
  passkeys as soon as it is touched; the button asks with nothing typed.
  While anybody here has a password, the field is also the password's,
  through `form=`, and the password folds under a disclosure. Closing the
  browser's sheet says nothing; only a refusal from the server does.
  Opened by its address, the page offers the node's name and
  `wabot-deploy sign-in-link`, not a button that cannot work.
- **A device asking** puts a dot on the account icon in the header, on
  every page, and the icon opens a panel — who, from where, with which
  passkey, and both answers — over the page rather than a notice pushing
  it down. `/account/live` sends the control again when it changes, so a
  request appears without reloading, opens the panel when it arrives
  while somebody watches, and disappears from every tab once one answers.
  The tab's title gets "(1)". Without scripting the dot is as of the page
  and the panel is a `popover`. Its "It was me" carries a verification
  without knowing the host (`ceremonies::begin_verify_anywhere`): the
  browser fills in its own, and the POST is checked where it lands.
- **The waiting device** numbers the three steps, takes the code as it is
  shown (`482 913`), counts down to the request's expiry, and leaves
  "Trust this device" unticked with why.
- **Your account** is one thing with tabs: its name above them and no
  breadcrumb on any, because `/account` had one crumb and
  `/account/devices` two and the page jumped between tabs. Lists at a
  reading width instead of tables. A passkey is named by the password
  manager that holds it, from the AAGUID against the community's list
  (`domain::values::passkey_provider`, names only), then by its browser;
  it can be renamed. Removing is a cross, not a bin — the password
  manager keeps its copy — and the last one asks first. The page tells
  the browser which passkeys it still knows (`signalAllAcceptedCredentials`).
  Times read in Spanish too.

A passkey made before 0086 has no AAGUID and keeps its browser's name:
only the ceremony that makes one carries it.

Two passkeys from one password manager on one account should not happen —
`excludeCredentials` lists the account's own — and the first node had
two from Safari four minutes apart. The list is right and decoded; what
Safari does with it when iCloud Keychain already holds one for the same
user is not known. The tell is "last used": if the older one stops being
used once the newer exists, Safari replaced it in the keychain and the
older row is dead. A device that refuses to make a second now says so.

Before it ships:

- On the VM with its CA trusted, and by Jorge in the browsers listed
  under "Verifying it". The QR on "Add a device" and the island's error
  line have not been looked at.
- Release notes: everybody is signed out once (the cookie's name
  changes), a password is deleted with its account's first passkey, and
  going back to `wabot-deploy.previous` afterwards needs its `passwd`.
