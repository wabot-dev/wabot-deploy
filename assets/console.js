// The console's two islands, registered with the framework runtime.
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

  wabot.island('copy', function (host) {
    var made = [];
    host.querySelectorAll('[data-copy]').forEach(function (block) {
      var button = document.createElement('button');
      button.type = 'button';
      button.className = 'btn btn-secondary btn-sm dsn-copy';
      button.textContent = block.dataset.copyLabel || 'Copy';
      button.addEventListener('click', function () {
        var text = block.textContent;
        var said = function () {
          var was = button.textContent;
          button.textContent = block.dataset.copiedLabel || 'Copied';
          window.setTimeout(function () {
            button.textContent = was;
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
        var detail = cell.querySelector('.failure, .tile-detail');
        if (detail) detail.textContent = state.detail;
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

    // Written only when it differs.
    //
    // Assigning the same text replaces the node's text anyway, and a
    // page doing that to seven cells a second reads as a page fidgeting:
    // reported from the memory view as "nothing changes but something
    // moves". Nothing here decides *what* to show — the server does
    // that — this only declines to repaint what is already right.
    var set = function (node, value) {
      if (node && node.textContent !== value) node.textContent = value;
    };

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
  wabot.island('logs-live', function (host, props) {
    if (!props || !props.project || !props.service) return null;
    var out = host.querySelector('[data-logs-out]');
    var state = host.querySelector('[data-logs-state]');
    if (!out) return null;

    // Where the page left off, from the markup rather than from props,
    // so a boosted navigation that swapped in a new panel follows that
    // panel's log and not the one before it.
    var from = out.getAttribute('data-from') || props.from || 0;
    var source = new EventSource(
      '/projects/' + encodeURIComponent(props.project) +
      '/services/' + encodeURIComponent(props.service) +
      '/logs/live?slot=' + encodeURIComponent(props.slot) +
      '&from=' + encodeURIComponent(from)
    );

    // The two words this can say arrive translated in the props: they
    // are server-rendered like every other string on the page, and a
    // script with English baked into it would be the one place the
    // console reverted.
    var say = function (words) {
      if (state) state.textContent = words;
    };
    say(props.following);

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

      // A deployment emptied the file, so what is on screen belongs to a
      // container that no longer exists. Appending to it would make one
      // run look like the continuation of another.
      if (data.restarted) {
        out.textContent = '';
      }
      if (!data.text) return;

      var stick = atEnd();
      out.appendChild(document.createTextNode(data.text));

      // A page left open for a day must not grow without limit. Trimmed
      // from the front, in whole lines, because half a line is worse
      // than a missing one.
      var LIMIT = 400000;
      if (out.textContent.length > LIMIT) {
        var kept = out.textContent.slice(-LIMIT);
        var cut = kept.indexOf('\n');
        out.textContent = cut === -1 ? kept : kept.slice(cut + 1);
      }
      if (stick) out.scrollTop = out.scrollHeight;
    };

    source.onerror = function () {
      // The browser reconnects on its own. Said because a log that has
      // silently stopped following looks exactly like a container that
      // has gone quiet, and those need telling apart.
      say(props.reconnecting);
    };
    source.onopen = function () {
      say(props.following);
    };

    // Start at the bottom: the newest line is the one somebody came for.
    out.scrollTop = out.scrollHeight;

    // Closed when the host leaves the DOM, or navigating away would
    // leave the stream open and coming back would open a second one.
    return function () {
      source.close();
    };
  });
})();
