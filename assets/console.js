// The console's islands, registered with the framework runtime.
//
// The console renders complete and works with this file absent: every
// form submits, every rule it enforces is enforced again on the
// server, and the node page shows a correct snapshot. What this adds
// is the noise removed — no certificate path asked of somebody who
// chose Let's Encrypt — and figures that keep up without a reload.
//
// ## Why `wabot.island` and not a loop over the document
//
// The framework's client runtime is loaded on every page and boosted
// navigation is on: an in-scope link swaps the view with
// `target.innerHTML = html` instead of loading a page. Two things
// follow, and both were bugs here before this file registered
// anything.
//
// A `<script>` that arrives inside swapped HTML **never executes** —
// that is what `innerHTML` does — so the node page's stream only ever
// started on a hard reload. And a listener attached once at module
// load is attached to forms that the next swap throws away.
//
// A registered island is re-hydrated after every swap and torn down
// when its host leaves the DOM, which is exactly the lifetime both of
// these want.

(function () {
  'use strict';

  // ---- conditional fields ---------------------------------------------
  //
  //   <div data-when="https">…</div>            shown while `https` is on
  //   <div data-when="renew_with=file">…</div>  shown while it reads `file`
  //   <input data-required-when="https">        required while that holds
  //
  // The markup says what depends on what, so a new dependent field is
  // an attribute rather than more script.
  //
  // A hidden field is still submitted. That is deliberate and the
  // server already relies on it: `add_port` ignores a hostname when the
  // HTTPS box is unticked, precisely so a name left in the field cannot
  // become a route nobody asked for.

  function parse(condition) {
    var parts = String(condition).split('=');
    return { name: parts[0], value: parts.length > 1 ? parts[1] : undefined };
  }

  // Radios share a name, so the answer is whichever one is checked; a
  // lone checkbox has no value worth reading, only a state.
  function holds(form, condition) {
    var controls = form.elements[condition.name];
    if (!controls) return false;
    var list = controls.length !== undefined && !controls.tagName ? [].slice.call(controls) : [controls];
    return list.some(function (control) {
      var togglable = control.type === 'checkbox' || control.type === 'radio';
      if (condition.value === undefined) {
        return togglable ? control.checked : control.value !== '';
      }
      return togglable
        ? control.checked && control.value === condition.value
        : control.value === condition.value;
    });
  }

  function applyTo(form) {
    form.querySelectorAll('[data-when]').forEach(function (node) {
      node.hidden = !holds(form, parse(node.dataset.when));
    });
    form.querySelectorAll('[data-required-when]').forEach(function (node) {
      // Never on a hidden field: a required control the operator cannot
      // see is a form that refuses to submit and will not say why.
      node.required = holds(form, parse(node.dataset.requiredWhen)) && !node.hidden;
    });
  }

  // ---- go to a thing by name -------------------------------------------
  //
  // The field in the header is a form that submits on its own. This adds
  // the keyboard's way to it and nothing else: the search is the
  // server's, so there is no list here to keep in step with it.

  wabot.island('go', function (host) {
    var field = host.querySelector('input[name="q"]');
    if (!field) return null;
    var onKey = function (event) {
      if ((event.metaKey || event.ctrlKey) && String(event.key).toLowerCase() === 'k') {
        event.preventDefault();
        field.focus();
        field.select();
      }
    };
    document.addEventListener('keydown', onKey);
    return function () {
      document.removeEventListener('keydown', onKey);
    };
  });

  wabot.island('fields', function (host) {
    var form = host.querySelector('form');
    if (!form) return null;
    var apply = function () {
      applyTo(form);
    };
    form.addEventListener('input', apply);
    form.addEventListener('change', apply);
    // Once before any interaction: the server rendered every field,
    // for a state the operator may not have chosen yet.
    apply();
    return function () {
      form.removeEventListener('input', apply);
      form.removeEventListener('change', apply);
    };
  });

  // ---- copying a connection string --------------------------------------
  //
  // The button is **not** in the markup: the server renders none, and this
  // puts one on each block it can copy. With scripting off there is no
  // button, which is the honest state — the string is selectable text
  // either way, and a button that cannot copy is a control that lies.
  //
  // An island rather than a listener at load, like everything else here: a
  // boosted navigation swaps the view and a listener attached once belongs
  // to markup that is gone.

  // The glyph, and the glyph that says it worked. Two overlapping sheets
  // and a tick — the same shape, stroke width and `currentColor` as every
  // other icon in this console, which is what keeps a button built here
  // from looking like it came from somewhere else.
  var COPY_GLYPH =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<rect x="9" y="9" width="11" height="11" rx="2"/>' +
    '<path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
  var DONE_GLYPH =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<polyline points="20 6 9 17 4 12"/></svg>';

  wabot.island('copy', function (host) {
    var made = [];
    host.querySelectorAll('[data-copy]').forEach(function (block) {
      var button = document.createElement('button');
      button.type = 'button';
      // No `btn-sm`: the height comes from `.dsn-copy`, which matches the
      // block beside it.
      button.className = 'btn btn-secondary btn-icon dsn-copy';
      // **The word becomes the accessible name, not the label.** An icon
      // on its own is a control with nothing to read out and nothing to
      // hover, so the translated string moves here rather than being
      // dropped — this is the one place the icon costs something, and it
      // is paid rather than skipped.
      var name = function (text) {
        button.setAttribute('aria-label', text);
        button.title = text;
      };
      name(block.dataset.copyLabel || 'Copy');
      button.innerHTML = COPY_GLYPH;
      button.addEventListener('click', function () {
        var text = block.textContent;
        var said = function () {
          var was = button.getAttribute('aria-label');
          name(block.dataset.copiedLabel || 'Copied');
          button.innerHTML = DONE_GLYPH;
          window.setTimeout(function () {
            name(was);
            button.innerHTML = COPY_GLYPH;
          }, 1200);
        };
        // The modern one needs a secure context, which a node reached by
        // IP on a self-signed certificate is not. So the fallback is not
        // legacy: it is the path this console is most often on.
        if (navigator.clipboard && window.isSecureContext) {
          navigator.clipboard.writeText(text).then(said, function () {
            select(block);
          });
          return;
        }
        select(block);
        said();
      });
      block.parentNode.appendChild(button);
      made.push(button);
    });

    return function () {
      made.forEach(function (button) {
        if (button.parentNode) button.parentNode.removeChild(button);
      });
    };
  });

  // Everything short of the clipboard: the string is selected, so one
  // keystroke finishes what the button started.
  var select = function (block) {
    var range = document.createRange();
    range.selectNodeContents(block);
    var selection = window.getSelection();
    if (!selection) return;
    selection.removeAllRanges();
    selection.addRange(range);
  };

  // ---- a project's service states ---------------------------------------
  //
  // Every badge in the table, replaced in place. Without this the row
  // still shows what the server rendered, which is correct and stale —
  // and a deployment finishing while somebody watches is exactly when
  // stale is worst.

  // Written only when it differs.
  //
  // Assigning the same text replaces the node's text anyway, and a page
  // doing that to seven cells a second reads as a page fidgeting —
  // reported from the memory view as "nothing changes but something
  // moves". Nothing here decides *what* to show; it only declines to
  // repaint what is already right.
  //
  // **Module scope, because two islands use it.** It was declared inside
  // `node-live` and called from `project-live`, which is a ReferenceError
  // the moment a message arrives — and one that takes the whole handler
  // with it, so a page kept its first paint and looked merely stale. A
  // helper shared by two islands belongs to neither.
  var set = function (node, value) {
    if (node && node.textContent !== value) node.textContent = value;
  };

  // A hostname carries dots, and a dot in a selector is a class. The
  // built-in is not everywhere yet, so this is the fallback rather than
  // the plan.
  var cssEscape = function (value) {
    if (window.CSS && window.CSS.escape) return window.CSS.escape(value);
    return String(value).replace(/[^a-zA-Z0-9_-]/g, function (ch) {
      return '\\' + ch;
    });
  };

  wabot.island('project-live', function (host, props) {
    if (!props || !props.project) return null;
    var source = new EventSource(
      '/projects/' + encodeURIComponent(props.project) + '/live'
    );

    source.onmessage = function (event) {
      var live;
      try {
        live = JSON.parse(event.data);
      } catch (e) {
        return;
      }

      // Every copy of every service, wherever it runs. A replica placed
      // on another node reports back seconds or minutes later, and this
      // row used to be true only at the instant the page rendered — so
      // "Waiting for that node" stayed on screen long after the node
      // had answered.
      var replicas = live.replicas || {};
      Object.keys(replicas).forEach(function (id) {
        var cell = host.querySelector('[data-replica="' + id + '"]');
        if (!cell) return;
        var state = replicas[id];
        var badge = cell.querySelector('.badge');
        if (badge) {
          badge.className = state.badge;
          var text = badge.lastChild;
          if (text) text.textContent = state.word;
          var dot = badge.querySelector('.dot');
          // The dot is markup the server may not have rendered — a
          // badge with no dot is a state that has none — so this only
          // ever restyles one that is there.
          if (dot) dot.className = state.dot;
        }
        // The line under the badge. Looked up by attribute rather than by
        // class, because the class is what changes: a copy that was fine
        // and then failed goes from `tile-detail` to `failure`, and
        // matching on the class would stop finding the element exactly
        // when it matters. The server renders it empty rather than
        // omitting it, or there would be nothing here to find.
        // The same reason, wherever else the page shows it. The service's
        // own card carries a copy of the copy-here's failure, and reading
        // it from this payload rather than adding a second field is what
        // keeps the two from disagreeing.
        // The service card's copy, which wants the *failure* only.
        //
        // `detail` is overloaded — its own doc says "an address while it
        // is up, a reason while it is not" — so writing it here put a
        // running container's address into an element styled as a
        // failure: a healthy service with a red band under it reading
        // `10.42.1.13`. Introduced by the fix that made this element
        // always present, and reported the same day.
        //
        // The table below can show either, because there the class
        // follows the badge. This one is red by definition, so it takes
        // the reason or nothing.
        var elsewhere = host.querySelector('[data-replica-detail="' + id + '"]');
        if (elsewhere) {
          var failing = state.badge && state.badge.indexOf('danger') !== -1;
          set(elsewhere, failing ? state.detail : '');
        }

        var detail = cell.querySelector('[data-detail]');
        if (detail) {
          set(detail, state.detail);
          var danger = state.badge && state.badge.indexOf('danger') !== -1;
          detail.className = (danger ? 'failure' : 'tile-detail') + ' detail-line';
        }

        // Its share of the machine, in its own cell rather than inside the
        // state one: the state is a badge and this is a number, and a
        // stream that wrote both into one place would make the badge's
        // markup this script's business.
        //
        // Only when the payload carries one. A copy on another node has
        // none, and the first tick of any stream has none — writing an
        // empty string over a figure would blank the column every time
        // somebody opened a second page.
        if (state.cpu) {
          var cpu = host.querySelector('[data-cpu="' + id + '"]');
          set(cpu, state.cpu);
        }
      });

      // A certificate arrives minutes after the hostname is saved, and
      // the badge saying so never changed on its own.
      var names = live.names || {};
      Object.keys(names).forEach(function (hostname) {
        var badge = host.querySelector('[data-name="' + cssEscape(hostname) + '"]');
        if (badge) badge.classList.toggle('is-hidden', !names[hostname].waiting);
      });

      // How far each edge instruction has got. A tick used to be the
      // end of what the page said: an errand went out, a name was
      // claimed and a certificate ordered, and none of it came back.
      var edges = live.edges || {};
      host.querySelectorAll('[data-edge]').forEach(function (badge) {
        var state = edges[badge.getAttribute('data-edge')];
        if (!state) {
          // Not chosen: the box is unticked and there is nothing to
          // report about it.
          badge.classList.add('is-hidden');
          return;
        }
        badge.className = state.badge;
        var dot = badge.querySelector('.dot');
        if (dot) dot.className = state.dot;
        var text = badge.lastChild;
        if (text) text.textContent = state.word;
      });

      var states = live.services || {};
      Object.keys(states).forEach(function (id) {
        var cell = host.querySelector('[data-state="' + id + '"]');
        if (!cell) return;
        var badge = cell.querySelector('.badge');
        var dot = cell.querySelector('.dot');
        if (!badge || !dot) return;
        // The word is the badge's last text node, after the dot.
        badge.className = states[id].badge;
        dot.className = states[id].dot;
        var text = badge.lastChild;
        if (text) text.textContent = states[id].word;

        // Both controls are in the markup; this shows the one that
        // applies. A row saying "Running" beside a Deploy button is the
        // page contradicting itself.
        //
        // The one that applies is always shown — disabled while a
        // deployment is in flight, never hidden. A control that
        // disappears takes the column's width with it, and leaves
        // nothing to read.
        // The address is assigned by the same message: a deployment
        // ends by giving the container one, and a row that showed the
        // new state beside the old address would be half-updated.
        var where = host.querySelector('[data-address="' + id + '"]');
        if (where) where.textContent = states[id].address;

        var row = cell.parentNode;
        ['deploy', 'stop'].forEach(function (name) {
          var form = row.querySelector('[data-action="' + name + '"]');
          if (!form) return;
          // The class, not the attribute — the server cannot spell
          // "no `hidden`" and this keeps both halves on one mechanism.
          form.classList.toggle('is-hidden', states[id].action !== name);
          var button = form.querySelector('button');
          if (button) button.disabled = !!states[id].busy;
        });
      });
    };

    return function () {
      source.close();
    };
  });

  // ---- an update, across the restart it performs -----------------------
  //
  // The only island here whose *disconnection* is part of the message.
  // Installing replaces this binary and restarts the node, so the socket
  // dies mid-install; `EventSource` reconnects on its own, and the first
  // payload after it comes back carries the version that answered.
  //
  // So the page says "the node is restarting" while it is gone and fills
  // the new version in when it returns — which is exactly what the note
  // telling somebody to reload was asking them to do by hand.
  wabot.island('updates-live', function (host) {
    var source = new EventSource('/updates/live');
    var waiting = host.querySelector('[data-run="waiting"]');
    var badge = host.querySelector('[data-run="badge"] .badge');
    var restarting = false;

    var put = function (key, value) {
      var node = host.querySelector('[data-run="' + key + '"]');
      if (node) node.textContent = value;
    };

    source.onerror = function () {
      // Every disconnection, not only the one an install causes — a
      // laptop that slept says the same thing and is equally true: this
      // page is not in touch with the node.
      restarting = true;
      if (badge) {
        badge.className = 'badge badge-info';
        var dot = badge.querySelector('.dot');
        if (dot) dot.className = 'dot dot-info dot-pulse';
        var text = badge.lastChild;
        if (text) text.textContent = 'Restarting';
      }
      if (waiting) {
        waiting.classList.remove('is-hidden');
        waiting.textContent = 'The node is not answering. This page reconnects on its own.';
      }
    };

    source.onmessage = function (event) {
      var run;
      try {
        run = JSON.parse(event.data);
      } catch (e) {
        return;
      }

      // The version always, because coming back with a new one is the
      // outcome the whole page is about.
      put('version', 'wabot-deploy ' + run.version);
      if (restarting && waiting) {
        waiting.textContent =
          'The node restarted and is answering as ' + run.version + '.';
      }
      restarting = false;
      if (run.none) return;

      if (badge) {
        badge.className = run.badge;
        var dot = badge.querySelector('.dot');
        if (dot) dot.className = run.dot;
        var text = badge.lastChild;
        if (text) text.textContent = run.word;
      }
      put('step', run.step);

      ['step-label', 'step'].forEach(function (key) {
        var node = host.querySelector('[data-run="' + key + '"]');
        if (node) node.classList.toggle('is-hidden', !run.in_flight || !run.step);
      });
      if (waiting && !restarting) {
        waiting.classList.toggle('is-hidden', !run.in_flight);
      }
    };

    return function () {
      source.close();
    };
  });

  // ---- the node's live figures ----------------------------------------
  //
  // Memory changes every second and a certificate request finishes
  // minutes after it starts. The server formats both — the same values
  // render the first paint and every update after it — so this only
  // assigns text and classes that are already on the page.

  wabot.island('node-live', function (host, props) {
    if (!props || !props.node) return null;
    var source = new EventSource('/nodes/' + encodeURIComponent(props.node) + '/live');

    var put = function (key, value) {
      set(host.querySelector('[data-cert="' + key + '"]'), value);
    };

    source.onmessage = function (event) {
      var data;
      try {
        data = JSON.parse(event.data);
      } catch (e) {
        return;
      }
      Object.keys(data.cells || {}).forEach(function (key) {
        set(host.querySelector('[data-cell="' + key + '"]'), data.cells[key]);
      });
      Object.keys(data.bars || {}).forEach(function (key) {
        var bar = host.querySelector('[data-bar="' + key + '"]');
        // A bare percentage, and only when it moved: the part carries a
        // width transition, so writing the same figure again would start
        // an animation from a value to itself.
        if (bar && bar.style.width !== data.bars[key]) {
          bar.style.width = data.bars[key];
        }
      });

      // What this node asked that one to do. A collection happens on a
      // fifteen-second timer and the answer comes back later still, so
      // "waiting to be collected" is exactly the badge somebody watches
      // after pressing the button.
      var errands = data.errands || {};
      Object.keys(errands).forEach(function (id) {
        var cell = host.querySelector('[data-errand="' + id + '"]');
        if (!cell) return;
        var state = errands[id];
        var badge = cell.querySelector('.badge');
        if (badge) {
          if (badge.className !== state.badge) badge.className = state.badge;
          set(badge.lastChild, state.word);
          var dot = badge.querySelector('.dot');
          if (dot && dot.className !== state.dot) dot.className = state.dot;
        }
        // A refusal is a paragraph the server renders only when there
        // is one, so this fills it when it exists and never invents it:
        // a reload brings the full row.
        set(cell.querySelector('.failure'), state.failure);
      });

      var cert = data.certificate;
      if (!cert) return;
      ['domain', 'issuer', 'renews', 'word', 'note', 'failure'].forEach(function (key) {
        put(key, cert[key]);
      });
      var badge = host.querySelector('[data-cert="badge"]');
      if (badge) badge.className = cert.badge;
      var dot = host.querySelector('[data-cert="dot"]');
      if (dot) dot.className = cert.dot;
    };

    // Closed when the host leaves the DOM. Without this, navigating
    // away from the node page would leave the stream open — and coming
    // back would open a second one.
    return function () {
      source.close();
    };
  });

  // What a container is saying, appended as it says it.
  //
  // The page already carries the window of the log it rendered — this
  // works with scripting off, which is the console's rule and matters
  // most here: somebody opens a log when a node is unwell. So this only
  // adds what arrives *after* that render, starting from the offset the
  // page was built at.
  var FULL_GLYPH =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/>' +
    '<path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/></svg>';
  var LEAVE_GLYPH =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" ' +
    'stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M8 3v3a2 2 0 0 1-2 2H3"/><path d="M21 8h-3a2 2 0 0 1-2-2V3"/>' +
    '<path d="M3 16h3a2 2 0 0 1 2 2v3"/><path d="M16 21v-3a2 2 0 0 1 2-2h3"/></svg>';

  // The window, not the browser's full screen: that is a decision of its
  // own, and the browser already has a key for it (Jorge, 2026-11-21).
  var expand = function (panel, tools, props) {
    var button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn btn-secondary btn-sm btn-icon';
    tools.appendChild(button);

    var log = panel.querySelector('[data-logs-end]');
    var show = function () {
      var full = panel.classList.contains('is-full');
      var words = full ? props.leave : props.full;
      button.innerHTML = full ? LEAVE_GLYPH : FULL_GLYPH;
      button.setAttribute('aria-label', words);
      button.title = words;
      button.setAttribute('aria-pressed', full ? 'true' : 'false');
    };
    var toggle = function (full) {
      var atEnd = log && log.scrollHeight - log.scrollTop - log.clientHeight < 32;
      panel.classList.toggle('is-full', full);
      show();
      if (atEnd) log.scrollTop = log.scrollHeight;
    };

    button.addEventListener('click', function () {
      toggle(!panel.classList.contains('is-full'));
    });
    var escape = function (event) {
      if (event.key === 'Escape' && panel.classList.contains('is-full')) toggle(false);
    };
    document.addEventListener('keydown', escape);
    show();

    return function () {
      document.removeEventListener('keydown', escape);
    };
  };

  // Each line drawn as the page draws it, so a run that starts while
  // somebody watches is set apart like the ones that were already there.
  // A line arrives in pieces; the piece without its end is drawn again
  // with the rest once the rest comes.
  var follow = function (out, state, props) {
    var stamp = new RegExp(props.stamp);
    var size = out.textContent.length;
    var whole = out.textContent.lastIndexOf('\n') + 1;
    var pending = out.textContent.slice(whole);

    var drop = function (count) {
      while (count > 0 && out.lastChild) {
        var node = out.lastChild;
        var length = node.textContent.length;
        if (length <= count) {
          out.removeChild(node);
          count -= length;
        } else {
          node.textContent = node.textContent.slice(0, length - count);
          count = 0;
        }
      }
    };

    var draw = function (text) {
      var drawn = document.createDocumentFragment();
      var first = !out.firstChild;
      var lines = text.match(/[^\n]*\n|[^\n]+$/g) || [];
      lines.forEach(function (line, index) {
        var words = line.replace(/\n$/, '');
        var end = line.slice(words.length);
        if (words.indexOf(props.boundary) === 0) {
          var run = document.createElement('span');
          run.className = first && index === 0 ? 'log-run log-run-first' : 'log-run';
          run.textContent = words;
          drawn.appendChild(run);
          if (end) drawn.appendChild(document.createTextNode(end));
          return;
        }
        var found = stamp.exec(line);
        if (found) {
          var quiet = document.createElement('span');
          quiet.className = 'log-stamp';
          quiet.textContent = found[0];
          drawn.appendChild(quiet);
          drawn.appendChild(document.createTextNode(line.slice(found[0].length)));
          return;
        }
        drawn.appendChild(document.createTextNode(line));
      });
      out.appendChild(drawn);
    };

    // A page left open for a day must not grow without limit. Trimmed
    // from the front, in whole lines, because half a line is worse
    // than a missing one.
    var LIMIT = 400000;
    var trim = function () {
      if (size <= LIMIT) return;
      var ended = true;
      while (out.firstChild && (size > LIMIT || !ended)) {
        var node = out.firstChild;
        var text = node.textContent;
        var cut = text.indexOf('\n');
        if (size <= LIMIT && node.nodeType === 3 && cut !== -1 && cut + 1 < text.length) {
          node.textContent = text.slice(cut + 1);
          size -= cut + 1;
          break;
        }
        out.removeChild(node);
        size -= text.length;
        ended = text.charAt(text.length - 1) === '\n';
      }
      var top = out.firstChild;
      if (top && top.classList && top.classList.contains('log-run')) {
        top.classList.add('log-run-first');
      }
      if (pending.length > size) {
        var left = out.textContent;
        pending = left.slice(left.lastIndexOf('\n') + 1);
      }
    };

    var source = new EventSource(
      '/projects/' + encodeURIComponent(props.project) +
      '/services/' + encodeURIComponent(props.service) +
      '/logs/live?slot=' + encodeURIComponent(props.slot) +
      '&from=' + encodeURIComponent(out.getAttribute('data-from') || props.from || 0)
    );

    // The two words this can say arrive translated in the props: they
    // are server-rendered like every other string on the page, and a
    // script with English baked into it would be the one place the
    // console reverted.
    var say = function (words) {
      if (state) state.textContent = words;
    };
    say(props.following);

    // Why the last stream ended, when the server could say: a copy on
    // another node that stopped answering. Kept until a stream opens
    // again, because the browser's own error comes straight after it and
    // would replace the reason with a bare "reconnecting".
    var trouble = '';
    var troubled = function () {
      say(trouble ? props.reconnecting + ' ' + trouble : props.reconnecting);
    };

    // Only when it is already at the bottom. Somebody who scrolled up to
    // read something must not be yanked back down by the next line.
    var atEnd = function () {
      return out.scrollHeight - out.scrollTop - out.clientHeight < 32;
    };

    source.onmessage = function (event) {
      var data;
      try {
        data = JSON.parse(event.data);
      } catch (e) {
        return;
      }

      if (data.trouble) {
        trouble = data.trouble;
        troubled();
        return;
      }

      // A deployment emptied the file, so what is on screen belongs to a
      // container that no longer exists. Appending to it would make one
      // run look like the continuation of another.
      if (data.restarted) {
        out.textContent = '';
        size = 0;
        pending = '';
      }
      if (!data.text) return;

      var stick = atEnd();
      drop(pending.length);
      var text = pending + data.text;
      draw(text);
      size += data.text.length;
      pending = text.slice(text.lastIndexOf('\n') + 1);
      trim();
      if (stick) out.scrollTop = out.scrollHeight;
    };

    source.onerror = function () {
      // The browser reconnects on its own. Said because a log that has
      // silently stopped following looks exactly like a container that
      // has gone quiet, and those need telling apart.
      troubled();
    };
    source.onopen = function () {
      trouble = '';
      say(props.following);
    };

    // Start at the bottom: the newest line is the one somebody came for.
    out.scrollTop = out.scrollHeight;

    return function () {
      source.close();
    };
  };

  wabot.island('logs-live', function (host, props) {
    props = props || {};
    // The end is the present, so a log opens there — on a reload, after a
    // swap, and for a copy on a node too old to follow.
    var end = host.querySelector('[data-logs-end]');
    if (end) end.scrollTop = end.scrollHeight;

    var undo = [];
    var panel = host.querySelector('[data-logs-panel]');
    var tools = host.querySelector('[data-logs-tools]');
    if (panel && tools && end && props.full) undo.push(expand(panel, tools, props));

    // Where the page left off is read from the markup rather than from
    // props, so a boosted navigation that swapped in a new panel follows
    // that panel's log and not the one before it.
    var out = host.querySelector('[data-logs-out]');
    if (out && props.project && props.service) {
      undo.push(follow(out, host.querySelector('[data-logs-state]'), props));
    }

    // Closed when the host leaves the DOM, or navigating away would
    // leave the stream open and coming back would open a second one.
    return function () {
      undo.forEach(function (done) {
        done();
      });
    };
  });

  // ---- a service's stability -------------------------------------------
  //
  // Everything here is already on the page: the cells carry their verdict,
  // the cards for each marker are rendered and hidden, and the numbers each
  // chart was drawn from come in the props. So this adds pointing and
  // asks the node for nothing.

  var BINARY = [
    ['GB', 1024 * 1024 * 1024],
    ['MB', 1024 * 1024],
    ['kB', 1024],
  ];

  var stabilityFormat = function (props) {
    var decimal = function (value, places) {
      var text = value.toFixed(places);
      return props.comma ? text.replace('.', ',') : text;
    };
    var bytes = function (value) {
      for (var i = 0; i < BINARY.length; i++) {
        var size = BINARY[i][1];
        if (value >= size) {
          var scaled = value / size;
          return (scaled < 10 ? decimal(scaled, 1) : Math.round(scaled)) + ' ' + BINARY[i][0];
        }
      }
      return Math.round(value) + ' B';
    };
    return function (unit, value) {
      if (value === null || value === undefined) return props.words.none;
      if (unit === 'bytes') return bytes(value);
      if (unit === 'rate') return bytes(value) + '/s';
      if (unit === 'millicores') return Math.round(value) + 'm';
      return String(value);
    };
  };

  var stabilityTip = function (parent) {
    var tip = document.createElement('div');
    tip.className = 'st-tip';
    tip.hidden = true;
    parent.appendChild(tip);
    return tip;
  };

  var row = function (tip, colour, label, value) {
    var line = document.createElement('div');
    line.className = 'st-tip-row';
    var name = document.createElement('span');
    name.className = 'st-tip-name';
    var key = document.createElement('i');
    key.className = colour;
    name.appendChild(key);
    name.appendChild(document.createTextNode(label));
    var number = document.createElement('b');
    number.textContent = value;
    line.appendChild(name);
    line.appendChild(number);
    tip.appendChild(line);
  };

  var place = function (tip, x, width) {
    if (x > width * 0.62) {
      tip.style.left = '';
      tip.style.right = width - x + 14 + 'px';
    } else {
      tip.style.right = '';
      tip.style.left = x + 14 + 'px';
    }
  };

  var two = function (n) {
    return (n < 10 ? '0' : '') + n;
  };
  var clock = function (ms) {
    var d = new Date(ms);
    return two(d.getUTCHours()) + ':' + two(d.getUTCMinutes());
  };

  wabot.island('stability', function (host, props) {
    props = props || {};
    var undo = [];
    var listen = function (node, type, handler) {
      node.addEventListener(type, handler);
      undo.push(function () {
        node.removeEventListener(type, handler);
      });
    };
    var format = stabilityFormat(props);

    var strips = host.querySelector('.st-strips');
    if (strips) {
      var cellTip = stabilityTip(strips);
      host.querySelectorAll('.st-cell[data-st-head]').forEach(function (cell) {
        var title = cell.getAttribute('title');
        cell.removeAttribute('title');
        var show = function () {
          cellTip.textContent = '';
          var when = document.createElement('div');
          when.className = 'st-tip-time';
          when.textContent = cell.dataset.stDay;
          var head = document.createElement('div');
          head.className = 'st-tip-head';
          var dot = document.createElement('i');
          dot.className = 'st-dot ' + cell.className.replace('st-cell', '').replace('sel', '').trim();
          head.appendChild(dot);
          head.appendChild(document.createTextNode(cell.dataset.stHead));
          var why = document.createElement('div');
          why.textContent = cell.dataset.stWhy;
          cellTip.appendChild(when);
          cellTip.appendChild(head);
          cellTip.appendChild(why);
          var box = strips.getBoundingClientRect();
          var at = cell.getBoundingClientRect();
          cellTip.style.top = at.bottom - box.top + 6 + 'px';
          place(cellTip, at.left - box.left + at.width / 2 - 14, box.width);
          cellTip.hidden = false;
        };
        var hide = function () {
          cellTip.hidden = true;
        };
        listen(cell, 'mouseenter', show);
        listen(cell, 'focus', show);
        listen(cell, 'mouseleave', hide);
        listen(cell, 'blur', hide);
        undo.push(function () {
          if (title) cell.setAttribute('title', title);
        });
      });
    }

    var lane = host.querySelector('[data-st-lane]');
    if (lane) {
      var cards = lane.querySelectorAll('[data-st-card]');
      var pinned = lane.querySelector('[data-st-pinned]');
      lane.querySelectorAll('[data-st-group]').forEach(function (marker) {
        var title = marker.getAttribute('title');
        marker.removeAttribute('title');
        var card = cards[Number(marker.dataset.stGroup)];
        if (!card) return;
        var show = function () {
          cards.forEach(function (one) {
            one.hidden = one !== card;
          });
        };
        var hide = function () {
          cards.forEach(function (one) {
            one.hidden = one !== pinned;
          });
        };
        listen(marker, 'mouseenter', show);
        listen(marker, 'focus', show);
        listen(marker, 'mouseleave', hide);
        listen(marker, 'blur', hide);
        undo.push(function () {
          if (title) marker.setAttribute('title', title);
        });
      });
    }

    var charts = host.querySelector('[data-st-charts]');
    if (charts && props.charts) {
      var full = 86400000 / props.step;
      var chartTip = stabilityTip(charts);
      var lines = [];
      host.querySelectorAll('[data-st-plot]').forEach(function (plot) {
        var cross = document.createElement('div');
        cross.className = 'st-cross';
        cross.hidden = true;
        plot.appendChild(cross);
        lines.push(cross);
        var dots = [];
        var chart = props.charts[Number(plot.dataset.stPlot)];
        if (!chart) return;
        var clear = function () {
          dots.forEach(function (dot) {
            if (dot.parentNode) dot.parentNode.removeChild(dot);
          });
          dots = [];
        };
        var dot = function (x, y, colour) {
          var mark = document.createElement('i');
          mark.className = 'st-pt ' + colour;
          mark.style.left = x + '%';
          mark.style.top = y + '%';
          plot.appendChild(mark);
          dots.push(mark);
        };
        var move = function (event) {
          var box = plot.getBoundingClientRect();
          var fraction = Math.max(0, Math.min(0.9999, (event.clientX - box.left) / box.width));
          var index = Math.floor(fraction * full);
          if (index >= props.buckets) {
            leave();
            return;
          }
          var x = ((index + 0.5) / full) * 100;
          lines.forEach(function (one) {
            one.hidden = false;
            one.style.left = x + '%';
          });
          clear();
          chartTip.textContent = '';
          var start = props.start + index * props.step;
          var time = document.createElement('div');
          time.className = 'st-tip-time mono';
          time.textContent = clock(start) + '–' + clock(start + props.step);
          chartTip.appendChild(time);
          if (chart.kind === 'lines') {
            var y = function (v) {
              return 100 - (Math.min(v, chart.top) / chart.top) * 100;
            };
            chart.series.forEach(function (values, copy) {
              var colour = 'c' + (copy < 4 ? copy + 1 : 0);
              var value = values[index];
              if (value !== null && value !== undefined) dot(x, y(value), colour);
              row(chartTip, 'st-line ' + colour, props.copies[copy] || '', format(chart.unit, value));
            });
            var ceiling = chart.ceil && chart.ceil[index];
            if (ceiling) row(chartTip, 'st-dash', props.words.ceiling, format(chart.unit, ceiling));
          } else {
            var up = chart.up[index];
            var down = chart.down[index];
            if (up !== null && up !== undefined) dot(x, 50 - (Math.min(up, chart.top) / chart.top) * 46, 'st-flow');
            if (down !== null && down !== undefined) dot(x, 50 + (Math.min(down, chart.top) / chart.top) * 46, 'st-flow');
            row(chartTip, 'st-line st-flow', chart.labels[0], format(chart.unit, up));
            row(chartTip, 'st-line st-flow', chart.labels[1], format(chart.unit, down));
          }
          (props.events || []).forEach(function (one) {
            var end = one.until || one.at;
            if (one.at < start + props.step && end >= start) {
              var said = document.createElement('div');
              said.className = 'st-tip-ev';
              said.textContent = clock(one.at) + ' ' + one.title;
              chartTip.appendChild(said);
            }
          });
          var outer = charts.getBoundingClientRect();
          chartTip.style.top = box.top - outer.top + 'px';
          place(chartTip, box.left - outer.left + (x / 100) * box.width, outer.width);
          chartTip.hidden = false;
        };
        var leave = function () {
          clear();
          lines.forEach(function (one) {
            one.hidden = true;
          });
          chartTip.hidden = true;
        };
        listen(plot, 'pointermove', move);
        listen(plot, 'pointerleave', leave);
        undo.push(clear);
      });
    }

    return function () {
      undo.forEach(function (done) {
        done();
      });
      host.querySelectorAll('.st-tip, .st-cross').forEach(function (made) {
        if (made.parentNode) made.parentNode.removeChild(made);
      });
    };
  });

  // A passkey made or given inside a form: the page rendered the options,
  // this fills hidden fields and submits it again. It fetches nothing.
  // `create` makes one, `get` signs in with one, `verify` proves who is
  // asking before something that makes a way in.
  wabot.island('passkey', function (host, props) {
    var form = host.querySelector('form[data-passkey]') || host.querySelector('form');
    if (!form || !props) return null;
    var error = host.querySelector('[data-passkey-error]');
    var say = function (text) {
      if (!error) return;
      error.textContent = text || '';
      if (text) error.removeAttribute('hidden');
      else error.setAttribute('hidden', '');
    };
    var decode = function (text) {
      var base = text.replace(/-/g, '+').replace(/_/g, '/');
      while (base.length % 4) base += '=';
      var binary = atob(base);
      var bytes = new Uint8Array(binary.length);
      for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      return bytes.buffer;
    };
    var encode = function (buffer) {
      var bytes = new Uint8Array(buffer);
      var binary = '';
      for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
      return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    };
    var creation = function (options, name) {
      var key = JSON.parse(JSON.stringify(options));
      key.challenge = decode(key.challenge);
      key.user.id = decode(key.user.id);
      if (name) {
        key.user.name = name;
        key.user.displayName = name;
      }
      (key.excludeCredentials || []).forEach(function (one) {
        one.id = decode(one.id);
      });
      return { publicKey: key };
    };
    var request = function (options) {
      var key = JSON.parse(JSON.stringify(options));
      key.challenge = decode(key.challenge);
      (key.allowCredentials || []).forEach(function (one) {
        one.id = decode(one.id);
      });
      return { publicKey: key };
    };
    var made = function (credential) {
      var response = credential.response;
      return JSON.stringify({
        id: credential.id,
        rawId: encode(credential.rawId),
        type: credential.type,
        response: {
          clientDataJSON: encode(response.clientDataJSON),
          attestationObject: encode(response.attestationObject),
          transports: response.getTransports ? response.getTransports() : []
        }
      });
    };
    var answered = function (credential) {
      var response = credential.response;
      return JSON.stringify({
        id: credential.id,
        rawId: encode(credential.rawId),
        type: credential.type,
        response: {
          clientDataJSON: encode(response.clientDataJSON),
          authenticatorData: encode(response.authenticatorData),
          signature: encode(response.signature),
          userHandle: response.userHandle ? encode(response.userHandle) : null
        }
      });
    };
    var put = function (name, value) {
      var input = form.querySelector('input[name="' + name + '"]');
      if (input) input.value = value;
    };
    // Closing the browser's sheet, or letting it time out, is somebody
    // changing their mind: it says nothing. A device that already holds a
    // passkey for the account is told so, because excludeCredentials is
    // what refused it and nothing else would explain the refusal.
    var failed = function (reason) {
      var name = reason && reason.name;
      if (name === 'NotAllowedError' || name === 'AbortError') return say('');
      if (name === 'InvalidStateError' && props.create) return say(props.exists);
      say(props.failed);
    };
    var supported = !!(window.PublicKeyCredential && navigator.credentials);
    var ready = false;
    var waiting = null;
    var send = function (submitter) {
      ready = true;
      if (form.requestSubmit) {
        form.requestSubmit(submitter && submitter.form === form ? submitter : undefined);
      } else {
        form.submit();
      }
    };

    // The username field offers this host's passkeys as soon as it is
    // touched: a request left waiting with `mediation: conditional`, which
    // the browser answers from its autofill. It is aborted before the
    // button asks, because a browser holds one request at a time.
    var field = props.autofill ? host.querySelector('input[autocomplete~="webauthn"]') : null;
    var stop = function () {
      if (waiting) {
        waiting.abort();
        waiting = null;
      }
    };
    var listen = function () {
      if (!field || !props.get || !supported || waiting) return;
      if (!PublicKeyCredential.isConditionalMediationAvailable) return;
      PublicKeyCredential.isConditionalMediationAvailable().then(function (available) {
        if (!available || waiting) return;
        waiting = new AbortController();
        var asked = request(props.get);
        asked.mediation = 'conditional';
        asked.signal = waiting.signal;
        navigator.credentials.get(asked).then(
          function (credential) {
            waiting = null;
            put('credential', answered(credential));
            send();
          },
          function () {
            waiting = null;
          }
        );
      }, function () {});
    };
    listen();

    // Enter in the username field asks for a passkey, unless a password
    // was typed: the field belongs to the password's form while there is
    // one, and a bare name would be refused there.
    var onKey = function (event) {
      if (event.key !== 'Enter') return;
      var password = host.querySelector('input[type="password"]');
      if (password && password.value) return;
      event.preventDefault();
      form.requestSubmit ? form.requestSubmit() : form.submit();
    };
    if (field) field.addEventListener('keydown', onKey);

    var submit = function (event) {
      if (ready) {
        ready = false;
        return;
      }
      if (!props.create && !props.get && !props.verify) return;
      if (!supported) {
        if (props.optional && !props.verify) return;
        event.preventDefault();
        say(props.unsupported);
        return;
      }
      event.preventDefault();
      stop();
      var submitter = event.submitter;
      say('');
      var steps = Promise.resolve();
      if (props.verify) {
        steps = steps.then(function () {
          return navigator.credentials.get(request(props.verify)).then(function (credential) {
            put('verify_credential', answered(credential));
          });
        });
      }
      if (props.get) {
        steps = steps.then(function () {
          return navigator.credentials.get(request(props.get)).then(function (credential) {
            put('credential', answered(credential));
          });
        });
      }
      if (props.create) {
        steps = steps.then(function () {
          var named = props.name ? form.querySelector('[name="' + props.name + '"]') : null;
          var name = named ? named.value.trim() : props.user || '';
          return navigator.credentials
            .create(creation(props.create, name))
            .then(function (credential) {
              put('credential', made(credential));
            });
        });
      }
      steps.then(
        function () {
          send(submitter);
        },
        function (reason) {
          failed(reason);
          listen();
        }
      );
    };
    form.addEventListener('submit', submit);

    // Tell the password manager which of this account's passkeys this
    // console still knows, so one removed here stops being offered.
    if (props.known && window.PublicKeyCredential && PublicKeyCredential.signalAllAcceptedCredentials) {
      PublicKeyCredential.signalAllAcceptedCredentials({
        rpId: props.known.rpId,
        userId: props.known.userId,
        allAcceptedCredentialIds: props.known.ids
      }).catch(function () {});
    }

    return function () {
      stop();
      form.removeEventListener('submit', submit);
      if (field) field.removeEventListener('keydown', onKey);
    };
  });

  // How long a code or a link has left, to the second. The page rendered
  // the minutes; this only counts them down, from when it arrived.
  wabot.island('countdown', function (host, props) {
    var out = host.querySelector('[data-countdown]');
    if (!out || !props) return null;
    var until = Date.now() + (props.left || 0) * 1000;
    var tick = function () {
      var left = Math.max(0, Math.round((until - Date.now()) / 1000));
      if (left === 0) {
        out.textContent = props.over;
        return false;
      }
      var seconds = left % 60;
      out.textContent = props.running.replace(
        '{}',
        Math.floor(left / 60) + ':' + (seconds < 10 ? '0' : '') + seconds
      );
      return true;
    };
    if (!tick()) return null;
    var timer = setInterval(function () {
      if (!tick()) clearInterval(timer);
    }, 1000);
    return function () {
      clearInterval(timer);
    };
  });

  // The account's icon in the header: a dot on every open page when a
  // device asks to sign in, gone from all of them once one answers. What
  // it swaps in is rendered by the server, forms and all; this decides
  // only when. A request that arrives while somebody watches opens the
  // panel, because the person approving is the one who just tried to sign
  // in on the other device.
  wabot.island('account-live', function (host, props) {
    var control = host.querySelector('[data-account-control]');
    if (!control || typeof EventSource === 'undefined') return null;
    var key = props.key;
    var asking = props.asking;
    var title = function () {
      var bare = document.title.replace(/^\(1\) /, '');
      document.title = asking ? '(1) ' + bare : bare;
    };
    title();
    var source = new EventSource('/account/live');
    source.onmessage = function (event) {
      var data;
      try {
        data = JSON.parse(event.data);
      } catch (e) {
        return;
      }
      if (data.key === key) return;
      var arrived = data.asking && data.asking !== asking;
      key = data.key;
      asking = data.asking;
      control.innerHTML = data.html;
      title();
      wabot.unmountRemoved();
      wabot.hydrateAll();
      var panel = control.querySelector('[popover]');
      if (arrived && panel && panel.showPopover) {
        try {
          panel.showPopover();
        } catch (e) {}
      }
    };
    source.onerror = function () {
      if (source.readyState === EventSource.CLOSED) source = null;
    };
    return function () {
      if (source) source.close();
      document.title = document.title.replace(/^\(1\) /, '');
    };
  });
})();
