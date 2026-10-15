#!/usr/bin/env python3
"""A browser with scripting off, for the console of the VM node.

Every form is read off the page it is on and posted with exactly the
fields a script-less browser would send — the checkboxes that are ticked,
the option that is selected, and the name and value of the button pressed.
That is the point: the console's rule is that it works without JavaScript,
and a test that hand-built its POST bodies would test a form nobody has.

    scripts/vm_console.py setup <token> <username> <password>
    scripts/vm_console.py signin <username> <password>
    scripts/vm_console.py get <path> [css-selector]
    scripts/vm_console.py forms <path>
    scripts/vm_console.py submit <page> <action-regex> [name=value …] [-name]

`-name` leaves a field out, which is how an unticked checkbox is sent.
Each identity keeps its own cookies: `WABOT_VM_AS=dev scripts/vm_console.py …`.
`WABOT_VM_HOST=org.wabot.test` reaches an organisation's console.

From Python, `import vm_console` and use `session`, `get`, `submit`, `text`
and `errors` — see `docs/vm-walk.md` for the walks written with them.

Needs `pip install requests beautifulsoup4`.
"""

import os
import pickle
import re
import sys
import warnings
from pathlib import Path

import requests
from bs4 import BeautifulSoup

warnings.filterwarnings("ignore")

BASE = os.environ.get("WABOT_VM_URL", "https://localhost:9443")
JARS = Path(os.environ.get("WABOT_VM_JARS", Path.home() / ".cache" / "wabot-vm-console"))


def session(who=None, host=None):
    who = who or os.environ.get("WABOT_VM_AS", "admin")
    JARS.mkdir(parents=True, exist_ok=True)
    s = requests.Session()
    s.verify = False
    s.jar_path = JARS / f"{who}.pkl"
    if s.jar_path.exists():
        with open(s.jar_path, "rb") as f:
            s.cookies.update(pickle.load(f))
    host = host or os.environ.get("WABOT_VM_HOST")
    if host:
        # The tenant console is chosen by `Host`, and the edge serves its
        # certificate by name — which is why `verify` is off here.
        s.headers["Host"] = host
    return s


def _save(s):
    with open(s.jar_path, "wb") as f:
        pickle.dump(s.cookies, f)


def get(s, path, follow=True):
    r = s.get(BASE + path, allow_redirects=follow)
    _save(s)
    return r


def post(s, path, data, follow=True):
    r = s.post(BASE + path, data=data, allow_redirects=follow)
    _save(s)
    return r


def soup(html):
    return BeautifulSoup(html, "html.parser")


def forms(html):
    return soup(html).find_all("form")


def fields_of(form):
    data = []
    for el in form.find_all(["input", "select", "textarea"]):
        name = el.get("name")
        if not name or el.has_attr("disabled"):
            continue
        if el.name == "input":
            kind = (el.get("type") or "text").lower()
            if kind in ("submit", "button", "image", "file", "reset"):
                continue
            if kind in ("checkbox", "radio"):
                if el.has_attr("checked"):
                    data.append((name, el.get("value", "on")))
                continue
            data.append((name, el.get("value", "")))
        elif el.name == "select":
            options = el.find_all("option")
            chosen = [o for o in options if o.has_attr("selected")] or options[:1]
            data.extend((name, o.get("value", o.text)) for o in chosen)
        else:
            data.append((name, el.text))
    return data


def find_form(html, action):
    return [f for f in forms(html) if re.search(action, f.get("action", ""))]


def options(html, action, field):
    """The values a `<select>` in that form offers."""
    found = find_form(html, action)
    if not found:
        return None
    select = found[0].find("select", attrs={"name": field})
    return [o.get("value") for o in select.find_all("option")] if select else None


def submit(s, page, action, overrides=None, drop=(), index=0, button=None, follow=True):
    """Load `page`, fill the form whose action matches, and post it."""
    r = get(s, page)
    found = find_form(r.text, action)
    if not found:
        actions = [f.get("action") for f in forms(r.text)]
        raise SystemExit(f"no form matching {action!r} on {page} [{r.status_code}]: {actions}")
    form = found[index]
    overrides = overrides or {}
    data = [(k, v) for k, v in fields_of(form) if k not in overrides and k not in drop]
    for k, v in overrides.items():
        if isinstance(v, list):
            data.extend((k, x) for x in v)
        elif v is not None:
            data.append((k, v))
    if button is not None:
        data.append(button)
    else:
        # The pressed button is part of what a browser sends: the theme
        # and language toggles are a name and a value on a `<button>`.
        for b in form.find_all(["button", "input"]):
            kind = (b.get("type") or ("submit" if b.name == "button" else "text")).lower()
            if kind == "submit" and b.get("name"):
                data.append((b["name"], b.get("value", "")))
                break
    target = form.get("action") or page
    url = target if target.startswith("http") else BASE + target
    if (form.get("method") or "get").lower() == "post":
        r = s.post(url, data=data, allow_redirects=follow)
    else:
        r = s.get(url, params=data, allow_redirects=follow)
    _save(s)
    return r


def text(html, selector=None):
    """What a person reads: no scripts, and nothing the page hides."""
    sp = soup(html)
    for t in sp(["script", "style"]):
        t.decompose()
    # `.is-hidden` is how a badge waits for its moment; reading it as
    # text reported "Certificate on the way" on a page that showed none.
    for t in sp.select(".is-hidden, [hidden]"):
        t.decompose()
    node = sp.select_one(selector) if selector else (sp.find("main") or sp.body or sp)
    return "" if node is None else re.sub(r"\n\s*\n+", "\n", node.get_text("\n", strip=True))


def errors(html):
    out = []
    for el in soup(html).select(".alert, .error, .form-error, [role=alert], .notice, .flash"):
        t = el.get_text(" ", strip=True)
        if t:
            out.append(t)
    return out


def show(r, selector=None, limit=6000):
    print(f"[{r.status_code}] {r.url}")
    for e in errors(r.text):
        print("  ALERT:", e)
    print(text(r.text, selector)[:limit])


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    s = session()
    verb, rest = argv[0], argv[1:]
    if verb == "setup":
        token, username, password = rest
        show(post(s, "/setup", {"setup_token": token, "username": username, "password": password}), limit=300)
    elif verb == "signin":
        username, password = rest
        show(post(s, "/sign-in", {"username": username, "password": password}), limit=300)
    elif verb == "get":
        show(get(s, rest[0]), rest[1] if len(rest) > 1 else None)
    elif verb == "forms":
        for f in forms(get(s, rest[0]).text):
            print(f.get("method", "get").upper(), f.get("action"), fields_of(f))
    elif verb == "submit":
        page, action, pairs = rest[0], rest[1], rest[2:]
        overrides = dict(p.split("=", 1) for p in pairs if "=" in p)
        drop = tuple(p[1:] for p in pairs if p.startswith("-"))
        show(submit(s, page, action, overrides, drop), limit=1500)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
