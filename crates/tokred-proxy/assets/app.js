/*
 * tokred dashboard (served by `tokred proxy` at /dashboard).
 *
 * How the UI is organised
 * -----------------------
 * - One page, no framework, no build step, no external request (the CSP allows only same-origin
 *   script, style and fetch/EventSource). Everything here is plain ES2020.
 * - SECURITY: every string that comes from the server (model output, tool results, prompts, ids)
 *   is rendered with textContent / createTextNode. Markup is never built from data and no
 *   HTML-parsing API is used anywhere in this file. Attribute values taken from data are
 *   whitelisted first (origin, role, status) and the only URLs are relative paths written here.
 * - State (`st`): `sessions` (id -> S, fed by GET /dashboard/api/sessions and `session` events),
 *   and, for the selected session only, `turns` (id -> T), `order` (turn ids, oldest first),
 *   `agents` (agent_id -> subagent event) and the follow/scroll flags.
 * - Data flow: the API snapshot and the SSE events all go through the same upsert functions
 *   (upsertSession, putTurn, onDelta, ...), which are idempotent: turns are keyed by id, a
 *   `turn_end` is authoritative and is never overwritten by a late `turn_start` or snapshot,
 *   and a `turn_delta` for an unknown turn creates a placeholder that `turn_start` completes.
 * - Rendering: the upserts only mark things dirty and call schedule(); flush() runs once per
 *   animation frame and updates the DOM in place (session list, summary, subagent strip, turn
 *   cards). Turn cards are cached per turn id, so a streaming delta touches one text node.
 *   At most MAX_RENDERED turns are in the DOM; older ones sit behind a "load earlier" control.
 * - Connection: one EventSource on /dashboard/events (unfiltered; events of sessions other than
 *   the selected one only update the left lists). The browser retries by itself; the header
 *   shows live / reconnecting, and after a reconnect the sessions list and the selected session
 *   are fetched again to resync.
 * - Follow: the timeline sticks to the bottom while following; scrolling up pauses it,
 *   scrolling back to the bottom resumes it; the "Follow" button turns it on or off.
 */
(function () {
  'use strict';

  var MAX_RENDERED = 300; // turn cards kept in the DOM
  var MAX_KEPT = 2000; // turns kept in memory per session
  var PREVIEW_CHARS = 2000; // text shown before "show more"
  var RECENT_LIMIT = 10;
  var PRUNE_SESSIONS_AT = 200;
  var RECONNECT_MS = 3000;
  var TICK_MS = 5000;
  var API = '/dashboard/api/sessions';
  var EVENTS = '/dashboard/events';

  var ORIGINS = ['main', 'subagent', 'auxiliary'];
  var ROLES = ['user', 'assistant', 'system'];
  var STATUSES = ['active', 'idle', 'ended'];

  var $ = function (id) { return document.getElementById(id); };
  var els = {
    conn: $('conn'),
    activeList: $('active-list'),
    recentList: $('recent-list'),
    activeCount: $('active-count'),
    recentCount: $('recent-count'),
    activeEmpty: $('active-empty'),
    recentEmpty: $('recent-empty'),
    placeholder: $('placeholder'),
    view: $('session-view'),
    summary: $('summary'),
    notice: $('notice'),
    subagents: $('subagents'),
    follow: $('follow'),
    scroll: $('timeline-scroll'),
    timeline: $('timeline'),
  };

  var st = {
    sessions: new Map(),
    sessionEls: new Map(),
    selectedId: null,
    selectToken: 0,
    turns: new Map(),
    order: [],
    turnMeta: new Map(),
    seq: 0,
    agents: new Map(),
    cards: new Map(),
    renderLimit: MAX_RENDERED,
    followPref: true,
    paused: false,
    lastScrollTop: 0,
    loadError: null,
    needsResync: false,
    es: null,
    dirty: { lists: true, summary: false, agents: false, structure: false, turns: new Set(), scroll: false },
    frame: 0,
  };

  // ---------- small helpers ----------

  function h(tag, className) {
    var el = document.createElement(tag);
    if (className) el.className = className;
    for (var i = 2; i < arguments.length; i++) {
      var c = arguments[i];
      if (c === null || c === undefined || c === false) continue;
      el.appendChild(typeof c === 'object' ? c : document.createTextNode(String(c)));
    }
    return el;
  }

  function num(v) {
    return typeof v === 'number' && isFinite(v) ? v : 0;
  }

  function str(v) {
    return typeof v === 'string' ? v : v === null || v === undefined ? '' : String(v);
  }

  function pick(v, allowed, fallback) {
    return allowed.indexOf(v) >= 0 ? v : fallback;
  }

  // Timestamps: unix seconds (number), unix milliseconds (number > 1e12) or an ISO string.
  function toMillis(v) {
    if (typeof v === 'number' && isFinite(v)) return v > 1e12 ? v : v * 1000;
    if (typeof v === 'string') {
      var p = Date.parse(v);
      return isNaN(p) ? 0 : p;
    }
    return 0;
  }

  function shortId(id) {
    var s = str(id);
    return s.length > 8 ? s.slice(0, 8) : s;
  }

  function fmtTokens(n) {
    n = num(n);
    if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
    if (n >= 1e4) return (n / 1e3).toFixed(1) + 'k';
    return String(Math.round(n));
  }

  function fmtCost(c) {
    if (typeof c !== 'number' || !isFinite(c)) return '-';
    return '$' + (c !== 0 && Math.abs(c) < 0.01 ? c.toFixed(4) : c.toFixed(2));
  }

  function fmtDuration(ms) {
    if (typeof ms !== 'number' || !isFinite(ms)) return '-';
    return ms < 1000 ? Math.round(ms) + ' ms' : (ms / 1000).toFixed(ms < 10000 ? 2 : 1) + ' s';
  }

  function fmtAgo(ms) {
    if (!ms) return '-';
    var s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 5) return 'now';
    if (s < 60) return s + 's ago';
    if (s < 3600) return Math.floor(s / 60) + 'm ago';
    if (s < 86400) return Math.floor(s / 3600) + 'h ago';
    return Math.floor(s / 86400) + 'd ago';
  }

  function fmtClock(ms) {
    return ms ? new Date(ms).toLocaleTimeString() : '';
  }

  function fmtDateTime(ms) {
    return ms ? new Date(ms).toLocaleString() : '-';
  }

  function shortModel(m) {
    return str(m).replace(/^claude-/, '');
  }

  function usageOf(t) {
    var u = (t && t.usage) || {};
    return { input: num(u.input), output: num(u.output), cache_write: num(u.cache_write), cache_read: num(u.cache_read) };
  }

  // ---------- scheduling ----------

  function schedule() {
    if (st.frame) return;
    st.frame = requestAnimationFrame(flush);
  }

  function flush() {
    st.frame = 0;
    var d = st.dirty;
    if (d.lists) {
      d.lists = false;
      renderLists();
    }
    if (st.selectedId !== null) {
      if (d.summary) {
        d.summary = false;
        renderSummary();
      }
      if (d.agents) {
        d.agents = false;
        renderAgents();
      }
      if (d.structure) {
        d.structure = false;
        renderStructure();
      }
      d.turns.forEach(function (key) {
        var card = st.cards.get(key);
        var t = st.turns.get(key);
        if (card && t) updateCard(card, t);
      });
      d.turns.clear();
      if (d.scroll) {
        d.scroll = false;
        if (st.followPref && !st.paused) scrollToBottom();
      }
    }
  }

  function scrollToBottom() {
    els.scroll.scrollTop = els.scroll.scrollHeight;
    st.lastScrollTop = els.scroll.scrollTop;
  }

  // ---------- network ----------

  function fetchJson(url) {
    return fetch(url, { headers: { Accept: 'application/json' }, cache: 'no-store' }).then(function (res) {
      if (!res.ok) throw new Error('HTTP ' + res.status);
      return res.json();
    });
  }

  function loadSessions() {
    return fetchJson(API)
      .then(function (data) {
        var all = [].concat((data && data.active) || [], (data && data.recent) || []);
        all.forEach(upsertSession);
        return true;
      })
      .catch(function () {
        return false;
      });
  }

  function loadDetail(id) {
    var token = st.selectToken;
    return fetchJson(API + '/' + encodeURIComponent(id))
      .then(function (data) {
        if (token !== st.selectToken || st.selectedId !== id) return;
        st.loadError = null;
        if (data && data.session) upsertSession(data.session);
        ((data && data.turns) || []).forEach(function (t) {
          putTurn(t, true);
        });
        markAll();
      })
      .catch(function (err) {
        if (token !== st.selectToken) return;
        st.loadError = 'Could not load this session (' + (err && err.message ? err.message : 'error') + ').';
        st.dirty.summary = true;
        schedule();
      });
  }

  function setConn(state) {
    els.conn.setAttribute('data-state', state);
    els.conn.textContent = state;
  }

  function connect() {
    var es = new EventSource(EVENTS);
    st.es = es;
    es.addEventListener('open', function () {
      setConn('live');
      if (st.needsResync) {
        st.needsResync = false;
        resync();
      }
    });
    es.addEventListener('error', function () {
      st.needsResync = true;
      setConn('reconnecting');
      if (es.readyState === 2) {
        // The browser gave up (for example a non-200 answer): retry ourselves.
        es.close();
        setTimeout(connect, RECONNECT_MS);
      }
    });
    on(es, 'session', function (s) {
      upsertSession(s);
    });
    on(es, 'turn_start', onTurnStart);
    on(es, 'turn_delta', onDelta);
    on(es, 'turn_end', onTurnEnd);
    on(es, 'subagent', onSubagent);
  }

  function on(es, name, fn) {
    es.addEventListener(name, function (e) {
      var data;
      try {
        data = JSON.parse(e.data);
      } catch (_) {
        return;
      }
      if (data && typeof data === 'object') fn(data);
    });
  }

  function resync() {
    loadSessions();
    if (st.selectedId !== null) loadDetail(st.selectedId);
  }

  // ---------- sessions ----------

  function upsertSession(s) {
    if (!s || (typeof s.id !== 'string' && typeof s.id !== 'number')) return;
    var id = String(s.id);
    var prev = st.sessions.get(id);
    if (prev && num(s.last_activity_at) < num(prev.last_activity_at)) return; // stale
    st.sessions.set(id, s);
    st.dirty.lists = true;
    if (id === st.selectedId) {
      st.dirty.summary = true;
      st.dirty.agents = true;
    }
    schedule();
  }

  function sortedSessions() {
    var active = [];
    var recent = [];
    st.sessions.forEach(function (s) {
      (s.status === 'active' ? active : recent).push(s);
    });
    var byActivity = function (a, b) {
      return num(b.last_activity_at) - num(a.last_activity_at);
    };
    active.sort(byActivity);
    recent.sort(byActivity);
    return { active: active, recent: recent.slice(0, RECENT_LIMIT) };
  }

  function pruneSessions(shown) {
    if (st.sessions.size <= PRUNE_SESSIONS_AT) return;
    var keep = new Set(shown.map(function (s) { return String(s.id); }));
    st.sessions.forEach(function (_, id) {
      if (!keep.has(id) && id !== st.selectedId) {
        st.sessions.delete(id);
        var e = st.sessionEls.get(id);
        if (e) e.li.remove();
        st.sessionEls.delete(id);
      }
    });
  }

  function renderLists() {
    var lists = sortedSessions();
    fillList(els.activeList, lists.active);
    fillList(els.recentList, lists.recent);
    els.activeCount.textContent = String(lists.active.length);
    els.recentCount.textContent = String(lists.recent.length);
    els.activeEmpty.hidden = lists.active.length > 0;
    els.recentEmpty.hidden = lists.recent.length > 0;
    pruneSessions(lists.active.concat(lists.recent));
    if (st.selectedId !== null) {
      st.dirty.summary = true;
      renderSummary();
      st.dirty.summary = false;
    }
  }

  function fillList(ul, sessions) {
    var wanted = sessions.map(function (s) {
      var id = String(s.id);
      var entry = st.sessionEls.get(id);
      if (!entry) {
        entry = createSessionEl(id);
        st.sessionEls.set(id, entry);
      }
      updateSessionEl(entry, s);
      return entry.li;
    });
    var focused = document.activeElement;
    var same = ul.children.length === wanted.length && wanted.every(function (li, i) { return ul.children[i] === li; });
    if (!same) {
      var keep = new Set(wanted);
      Array.prototype.slice.call(ul.children).forEach(function (c) {
        if (!keep.has(c)) c.remove();
      });
      wanted.forEach(function (li, i) {
        if (ul.children[i] !== li) ul.insertBefore(li, ul.children[i] || null);
      });
      if (focused && focused !== document.activeElement && document.contains(focused)) focused.focus();
    }
  }

  function createSessionEl(id) {
    var dot = h('span', 'dot');
    dot.setAttribute('aria-hidden', 'true');
    var sid = h('span', 'sid');
    var status = h('span', 'sstatus');
    var ago = h('span', 'ago');
    var models = h('span', 'models');
    var stats = h('span', 'sstats');
    var btn = h('button', 'session', h('span', 'row', dot, sid, status, ago), h('span', 'row', models), h('span', 'row', stats));
    btn.type = 'button';
    btn.addEventListener('click', function () {
      select(id);
    });
    var entry = { li: h('li', null, btn), btn: btn, dot: dot, sid: sid, status: status, ago: ago, models: models, stats: stats, last: 0 };
    return entry;
  }

  function sessionStats(s) {
    var parts = [fmtCost(s.cost_usd), num(s.requests) + ' req'];
    if (num(s.subagents_running) > 0) parts.push(num(s.subagents_running) + ' subagent' + (num(s.subagents_running) === 1 ? '' : 's') + ' running');
    return parts.join(' · ');
  }

  function updateSessionEl(entry, s) {
    var status = pick(s.status, STATUSES, 'ended');
    entry.dot.setAttribute('data-status', status);
    entry.sid.textContent = shortId(s.id);
    entry.sid.title = str(s.id);
    entry.status.textContent = status;
    entry.last = toMillis(s.last_activity_at);
    entry.ago.textContent = fmtAgo(entry.last);
    var models = Array.isArray(s.models) ? s.models.map(shortModel).join(', ') : '';
    entry.models.textContent = models || 'no model yet';
    entry.stats.textContent = sessionStats(s);
    var selected = String(s.id) === st.selectedId;
    if (selected) entry.btn.setAttribute('aria-current', 'true');
    else entry.btn.removeAttribute('aria-current');
  }

  function tickAgo() {
    st.sessionEls.forEach(function (e) {
      e.ago.textContent = fmtAgo(e.last);
    });
    if (st.selectedId !== null) {
      st.dirty.summary = true;
      schedule();
    }
  }

  // ---------- selection ----------

  function select(id) {
    if (id === st.selectedId) return;
    st.selectedId = id;
    st.selectToken++;
    st.turns = new Map();
    st.order = [];
    st.turnMeta = new Map();
    st.agents = new Map();
    st.cards = new Map();
    st.renderLimit = MAX_RENDERED;
    st.paused = false;
    st.loadError = null;
    st.dirty.turns.clear();
    els.timeline.textContent = '';
    els.placeholder.hidden = true;
    els.view.hidden = false;
    setHash(id);
    markAll();
    loadDetail(id);
  }

  function markAll() {
    st.dirty.lists = true;
    st.dirty.summary = true;
    st.dirty.agents = true;
    st.dirty.structure = true;
    st.dirty.scroll = true;
    schedule();
  }

  function setHash(id) {
    try {
      history.replaceState(null, '', '#session=' + encodeURIComponent(id));
    } catch (_) {
      /* ignore: the hash is only a convenience */
    }
  }

  function hashSession() {
    var m = /^#session=(.+)$/.exec(location.hash);
    if (!m) return null;
    try {
      return decodeURIComponent(m[1]);
    } catch (_) {
      return null;
    }
  }

  // ---------- turns ----------

  function mergeTurn(ex, inc) {
    if (!ex) return inc;
    if (!inc.in_flight) return inc; // turn_end (or an ended snapshot) is authoritative
    if (!ex.in_flight) return ex; // never go back from ended to in flight
    var out = Object.assign({}, inc);
    var exLen = ex.response ? str(ex.response.text).length : 0;
    var incLen = inc.response ? str(inc.response.text).length : 0;
    if (ex.response && exLen >= incLen) out.response = ex.response;
    return out;
  }

  function putTurn(t, fromSnapshot) {
    if (!t || (typeof t.id !== 'string' && typeof t.id !== 'number')) return;
    if (t.session_id !== undefined && String(t.session_id) !== st.selectedId) return;
    var key = String(t.id);
    var ex = st.turns.get(key);
    var merged = mergeTurn(ex, t);
    st.turns.set(key, merged);
    if (!ex) {
      var ms = toMillis(merged.ts);
      st.turnMeta.set(key, { ms: ms, seq: st.seq++ });
      var i = st.order.length;
      while (i > 0 && st.turnMeta.get(st.order[i - 1]).ms > ms) i--;
      st.order.splice(i, 0, key);
      if (st.order.length > MAX_KEPT) {
        var gone = st.order.shift();
        st.turns.delete(gone);
        st.turnMeta.delete(gone);
        st.cards.delete(gone);
      }
      st.dirty.structure = true;
      st.dirty.agents = true;
    }
    st.dirty.turns.add(key);
    st.dirty.scroll = true;
    if (!fromSnapshot) schedule();
  }

  function onTurnStart(t) {
    putTurn(Object.assign({}, t, { in_flight: true }), false);
  }

  function onTurnEnd(t) {
    putTurn(Object.assign({}, t, { in_flight: false }), false);
  }

  function onDelta(d) {
    if (String(d.session_id) !== st.selectedId || typeof d.text !== 'string') return;
    var key = String(d.turn_id);
    var t = st.turns.get(key);
    if (!t) {
      putTurn({ id: d.turn_id, session_id: d.session_id, ts: Date.now() / 1000, in_flight: true, response: null }, false);
      t = st.turns.get(key);
    }
    if (!t || !t.in_flight) return; // a late delta after turn_end
    if (!t.response) t.response = { text: '', thinking: null, tool_uses: [] };
    t.response.text = str(t.response.text) + d.text;
    st.dirty.turns.add(key);
    st.dirty.scroll = true;
    schedule();
  }

  function onSubagent(a) {
    if (String(a.session_id) !== st.selectedId || a.agent_id === undefined || a.agent_id === null) return;
    st.agents.set(String(a.agent_id), a);
    st.dirty.agents = true;
    schedule();
  }

  // ---------- summary / notice / subagents ----------

  // `optional` stats are hidden on narrow windows to leave room for the timeline.
  function stat(label, value, optional) {
    return h('span', optional ? 'stat optional' : 'stat', h('span', 'label', label), h('span', 'value', value));
  }

  function renderSummary() {
    var s = st.sessions.get(st.selectedId);
    els.summary.textContent = '';
    if (!s) {
      els.summary.appendChild(h('span', 'title', shortId(st.selectedId)));
    } else {
      var status = pick(s.status, STATUSES, 'ended');
      var dot = h('span', 'dot');
      dot.setAttribute('data-status', status);
      dot.setAttribute('aria-hidden', 'true');
      var title = h('span', 'title', dot, h('span', null, str(s.id)), h('span', 'muted', status));
      var inputs = num(s.input) + num(s.cache_read) + num(s.cache_write);
      var hit = inputs > 0 ? Math.round((num(s.cache_read) / inputs) * 100) + '%' : '-';
      var models = Array.isArray(s.models) ? s.models.map(shortModel).join(', ') : '';
      [
        title,
        stat('Cost', fmtCost(s.cost_usd)),
        stat('Requests', String(num(s.requests))),
        stat('Input', fmtTokens(s.input)),
        stat('Output', fmtTokens(s.output)),
        stat('Cache read', fmtTokens(s.cache_read)),
        stat('Cache write', fmtTokens(s.cache_write)),
        stat('Cache hit', hit),
        stat('Models', models || '-', true),
        stat('Started', fmtDateTime(toMillis(s.started_at)), true),
        stat('Last activity', fmtAgo(toMillis(s.last_activity_at)), true),
      ].forEach(function (n) {
        els.summary.appendChild(n);
      });
    }
    renderNotice(s);
  }

  function renderNotice(s) {
    els.notice.textContent = '';
    if (st.loadError) {
      els.notice.appendChild(document.createTextNode(st.loadError));
      els.notice.hidden = false;
    } else if (s && s.content_recorded === false) {
      els.notice.appendChild(document.createTextNode('Message content is not recorded for this session; enable '));
      els.notice.appendChild(h('code', null, '[proxy] log_bodies'));
      els.notice.appendChild(document.createTextNode(' to see what was sent to Claude and what came back.'));
      els.notice.hidden = false;
    } else {
      els.notice.hidden = true;
    }
  }

  function computeAgents() {
    var map = new Map();
    st.turns.forEach(function (t) {
      if (t.agent_id === null || t.agent_id === undefined) return;
      var key = String(t.agent_id);
      var prev = map.get(key);
      var running = prev && prev.state === 'running' ? true : !!t.in_flight;
      map.set(key, { agent_id: key, agent_type: t.agent_type || (prev && prev.agent_type) || null, state: running ? 'running' : 'seen', ts: t.ts });
    });
    st.agents.forEach(function (a, key) {
      var prev = map.get(key);
      map.set(key, { agent_id: key, agent_type: a.agent_type || (prev && prev.agent_type) || null, state: pick(a.state, ['running', 'finished'], 'seen'), ts: a.ts });
    });
    var rank = { running: 0, seen: 1, finished: 2 };
    return Array.from(map.values()).sort(function (a, b) {
      return rank[a.state] - rank[b.state] || toMillis(b.ts) - toMillis(a.ts);
    });
  }

  function renderAgents() {
    var agents = computeAgents();
    els.subagents.textContent = '';
    els.subagents.hidden = agents.length === 0;
    if (agents.length === 0) return;
    els.subagents.appendChild(h('span', 'label', 'Subagents'));
    agents.forEach(function (a) {
      els.subagents.appendChild(
        h('span', 'agent-chip', h('span', 'atype', a.agent_type || 'agent'), h('span', 'aid', shortId(a.agent_id)), h('span', 'astate', a.state))
      );
      els.subagents.lastChild.setAttribute('data-state', a.state);
      els.subagents.lastChild.title = a.agent_id;
    });
  }

  // ---------- timeline ----------

  function textBlock() {
    var pre = h('pre', 'text');
    pre.setAttribute('aria-live', 'off');
    var btn = h('button', 'more');
    btn.type = 'button';
    btn.hidden = true;
    var el = h('div', 'textblock', pre, btn);
    var state = { full: '', expanded: false, tail: false };

    function paint() {
      var full = state.full;
      var long = full.length > PREVIEW_CHARS;
      var shown = full;
      if (long && !state.expanded) {
        shown = state.tail ? '…' + full.slice(full.length - PREVIEW_CHARS) : full.slice(0, PREVIEW_CHARS);
      }
      pre.textContent = shown;
      btn.hidden = !long;
      btn.textContent = state.expanded ? 'Show less' : 'Show more (' + Math.max(0, full.length - PREVIEW_CHARS) + ' more characters)';
      if (state.tail && !state.expanded) pre.scrollTop = pre.scrollHeight;
    }

    btn.addEventListener('click', function () {
      state.expanded = !state.expanded;
      paint();
    });
    return {
      el: el,
      set: function (text, tail) {
        if (text === state.full && tail === state.tail) return;
        state.full = text;
        state.tail = tail;
        paint();
      },
    };
  }

  function createCard(key) {
    var badge = h('span', 'badge');
    var agent = h('span', 'agent');
    var model = h('span', 'model');
    var status = h('span', 'status');
    var stop = h('span', 'stop');
    var time = h('span', 'time');
    var meta = h('div', 'turn-meta');
    var chips = h('div', 'turn-chips');
    var routing = h('div', 'routing');
    var hint = h('p', 'hint');

    var sentBody = h('div', 'msgs');
    var sentSummary = h('summary', null, 'Sent');
    var sent = h('details', 'sent', sentSummary, sentBody);

    var respText = textBlock();
    var thinkingText = textBlock();
    var thinkingSummary = h('summary', null, 'Thinking');
    var thinking = h('details', 'thinking', thinkingSummary, thinkingText.el);
    var toolUses = h('ul', 'tool-uses');
    var respWait = h('p', 'hint');
    var respSummary = h('summary', null, 'Response');
    var resp = h('details', 'resp', respSummary, respWait, respText.el, thinking, toolUses);

    var el = h('article', 'turn', h('div', 'turn-head', badge, agent, model, status, stop, time), meta, chips, routing, hint, sent, resp);
    var card = {
      key: key,
      el: el,
      badge: badge,
      agent: agent,
      model: model,
      status: status,
      stop: stop,
      time: time,
      meta: meta,
      chips: chips,
      routing: routing,
      hint: hint,
      sent: sent,
      sentSummary: sentSummary,
      sentBody: sentBody,
      sentSig: null,
      resp: resp,
      respWait: respWait,
      respText: respText,
      thinking: thinking,
      thinkingText: thinkingText,
      toolUses: toolUses,
      toolSig: null,
      chipSig: null,
      opened: false,
    };
    return card;
  }

  function updateCard(card, t) {
    var origin = pick(t.origin, ORIGINS, 'main');
    var inFlight = !!t.in_flight;
    var ok = typeof t.status === 'number' ? t.status >= 200 && t.status < 400 : true;
    card.el.setAttribute('data-origin', origin);
    card.el.setAttribute('data-inflight', inFlight ? 'true' : 'false');
    card.el.setAttribute('data-failed', !inFlight && !ok ? 'true' : 'false');

    card.badge.textContent = origin;
    card.badge.setAttribute('data-origin', origin);
    card.agent.textContent = origin === 'subagent' || t.agent_id ? str(t.agent_type || 'agent') + (t.agent_id ? ' ' + shortId(t.agent_id) : '') : '';
    card.agent.hidden = !card.agent.textContent;
    var requested = str(t.model_requested);
    var effective = str(t.model_effective);
    card.model.textContent =
      effective && requested && effective !== requested ? shortModel(requested) + ' → ' + shortModel(effective) : shortModel(effective || requested);
    if (inFlight) {
      card.status.textContent = '';
      card.status.className = 'status state-badge';
      card.status.appendChild(document.createTextNode('streaming'));
    } else {
      card.status.className = 'status';
      card.status.setAttribute('data-ok', ok ? 'true' : 'false');
      card.status.textContent = typeof t.status === 'number' ? String(t.status) : '-';
    }
    card.stop.textContent = t.stop_reason ? 'stop: ' + str(t.stop_reason) : '';
    card.stop.hidden = !t.stop_reason;
    var ms = toMillis(t.ts);
    card.time.textContent = fmtClock(ms);
    card.time.title = fmtDateTime(ms);

    var u = usageOf(t);
    var metaParts = [];
    metaParts.push('duration ' + fmtDuration(t.duration_ms));
    metaParts.push('TTFB ' + fmtDuration(t.ttfb_ms));
    metaParts.push('in ' + fmtTokens(u.input) + ' · out ' + fmtTokens(u.output) + ' · cache read ' + fmtTokens(u.cache_read) + ' · cache write ' + fmtTokens(u.cache_write));
    metaParts.push('cost ' + fmtCost(t.cost_usd));
    card.meta.textContent = metaParts.join('  |  ');

    var tools = Array.isArray(t.tools) ? t.tools.map(str) : [];
    var chipSig = tools.join('\u0000');
    if (chipSig !== card.chipSig) {
      card.chipSig = chipSig;
      card.chips.textContent = '';
      tools.forEach(function (name) {
        card.chips.appendChild(h('span', 'chip', name));
      });
    }

    card.routing.textContent = '';
    card.routing.hidden = !t.routing;
    if (t.routing) {
      card.routing.appendChild(document.createTextNode('routing: '));
      card.routing.appendChild(h('span', 'outcome', str(t.routing.outcome)));
      if (t.routing.reason) card.routing.appendChild(document.createTextNode(' — ' + str(t.routing.reason)));
    }

    updateContent(card, t, inFlight);
  }

  function updateContent(card, t, inFlight) {
    var session = st.sessions.get(st.selectedId);
    var recorded = !(session && session.content_recorded === false);
    var hasSent = Array.isArray(t.sent);
    var hasResp = !!t.response;
    var none = !hasSent && !hasResp && !recorded;

    card.hint.hidden = !none;
    card.hint.textContent = none ? 'Content is not recorded; enable `[proxy] log_bodies` to see it.' : '';
    card.sent.hidden = none || !hasSent;
    card.resp.hidden = none;

    if (hasSent) {
      var sig = t.sent
        .map(function (m) {
          return str(m && m.role) + ':' + str(m && m.text).length;
        })
        .join(',');
      if (sig !== card.sentSig) {
        card.sentSig = sig;
        card.sentSummary.textContent = 'Sent (' + t.sent.length + ' new message' + (t.sent.length === 1 ? '' : 's') + ')';
        card.sentBody.textContent = '';
        if (t.sent.length === 0) card.sentBody.appendChild(h('p', 'hint', 'Nothing new was sent in this turn.'));
        t.sent.forEach(function (m) {
          var block = textBlock();
          block.set(str(m && m.text), false);
          var role = pick(m && m.role, ROLES, null);
          var label = h('span', 'role', role || str(m && m.role) || 'message');
          var msg = h('div', 'msg', label, block.el);
          if (role) msg.setAttribute('data-role', role);
          card.sentBody.appendChild(msg);
        });
      }
    }

    if (!none) {
      var r = t.response;
      card.respText.el.hidden = !(r && str(r.text));
      card.respText.set(r ? str(r.text) : '', inFlight);
      card.respWait.hidden = !!(r && str(r.text)) || !!(r && Array.isArray(r.tool_uses) && r.tool_uses.length);
      card.respWait.textContent = inFlight ? 'Waiting for the response…' : '(no text in the response)';
      var thinking = r && r.thinking ? str(r.thinking) : '';
      card.thinking.hidden = !thinking;
      if (thinking) card.thinkingText.set(thinking, inFlight);
      var uses = r && Array.isArray(r.tool_uses) ? r.tool_uses : [];
      var toolSig = uses
        .map(function (u) {
          return str(u && u.name) + ':' + str(u && u.input_summary);
        })
        .join('\u0000');
      if (toolSig !== card.toolSig) {
        card.toolSig = toolSig;
        card.toolUses.textContent = '';
        uses.forEach(function (u) {
          card.toolUses.appendChild(h('li', null, h('span', 'tname', str(u && u.name)), ' ', h('span', 'tinput', str(u && u.input_summary))));
        });
      }
      if (!card.opened) {
        card.opened = true;
        if (inFlight) card.resp.open = true; // follow the live response by default
      }
    }
  }

  function loadEarlierButton() {
    var btn = h('button', 'toggle load-earlier');
    btn.type = 'button';
    btn.addEventListener('click', function () {
      st.renderLimit += MAX_RENDERED;
      st.dirty.structure = true;
      schedule();
      if (st.selectedId !== null) loadDetail(st.selectedId);
    });
    return btn;
  }

  var earlierBtn = loadEarlierButton();

  function renderStructure() {
    var visible = st.order.slice(-st.renderLimit);
    var hidden = st.order.length - visible.length;
    var wanted = [];
    if (hidden > 0) {
      earlierBtn.textContent = 'Load earlier turns (' + hidden + ' hidden)';
      wanted.push(earlierBtn);
    }
    var keep = new Set(visible);
    st.cards.forEach(function (_, key) {
      if (!keep.has(key)) st.cards.delete(key);
    });
    visible.forEach(function (key) {
      var card = st.cards.get(key);
      if (!card) {
        card = createCard(key);
        st.cards.set(key, card);
        updateCard(card, st.turns.get(key));
      }
      wanted.push(card.el);
    });
    var wantedSet = new Set(wanted);
    Array.prototype.slice.call(els.timeline.children).forEach(function (c) {
      if (!wantedSet.has(c)) c.remove();
    });
    wanted.forEach(function (el, i) {
      if (els.timeline.children[i] !== el) els.timeline.insertBefore(el, els.timeline.children[i] || null);
    });
  }

  // ---------- follow / scroll ----------

  function renderFollow() {
    var label = !st.followPref ? 'off' : st.paused ? 'paused' : 'on';
    els.follow.textContent = 'Follow: ' + label;
    els.follow.setAttribute('aria-pressed', st.followPref ? 'true' : 'false');
  }

  els.follow.addEventListener('click', function () {
    st.followPref = !st.followPref;
    st.paused = false;
    renderFollow();
    if (st.followPref) scrollToBottom();
  });

  els.scroll.addEventListener('scroll', function () {
    var box = els.scroll;
    var atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
    if (atBottom) {
      if (st.paused) {
        st.paused = false;
        renderFollow();
      }
    } else if (st.followPref && !st.paused && box.scrollTop < st.lastScrollTop) {
      st.paused = true;
      renderFollow();
    }
    st.lastScrollTop = box.scrollTop;
  });

  // ---------- boot ----------

  function boot() {
    renderFollow();
    connect();
    loadSessions().then(function () {
      var wanted = hashSession();
      if (wanted !== null && st.sessions.has(wanted)) {
        select(wanted);
        return;
      }
      var first = sortedSessions().active[0];
      if (st.selectedId === null && first) select(String(first.id));
    });
    setInterval(tickAgo, TICK_MS);
  }

  boot();
})();
