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

  // ---- a project's service states ---------------------------------------
  //
  // Every badge in the table, replaced in place. Without this the row
  // still shows what the server rendered, which is correct and stale —
  // and a deployment finishing while somebody watches is exactly when
  // stale is worst.

  wabot.island('project-live', function (host, props) {
    if (!props || !props.project) return null;
    var source = new EventSource(
      '/projects/' + encodeURIComponent(props.project) + '/live'
    );

    source.onmessage = function (event) {
      var states;
      try {
        states = JSON.parse(event.data);
      } catch (e) {
        return;
      }
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
        var row = cell.parentNode;
        ['deploy', 'stop'].forEach(function (name) {
          var form = row.querySelector('[data-action="' + name + '"]');
          if (!form) return;
          form.hidden = states[id].action !== name;
          var button = form.querySelector('button');
          if (button) button.disabled = !!states[id].busy;
        });
      });
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
      var node = host.querySelector('[data-cert="' + key + '"]');
      if (node) node.textContent = value;
    };

    source.onmessage = function (event) {
      var data;
      try {
        data = JSON.parse(event.data);
      } catch (e) {
        return;
      }
      Object.keys(data.cells || {}).forEach(function (key) {
        var cell = host.querySelector('[data-cell="' + key + '"]');
        if (cell) cell.textContent = data.cells[key];
      });
      Object.keys(data.bars || {}).forEach(function (key) {
        var bar = host.querySelector('[data-bar="' + key + '"]');
        if (bar) bar.style.width = data.bars[key];
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
})();
