/* IPTV for Samsung TVs. All channel logic lives in the shared Rust core
 * (crates/iptv-web, compiled to WebAssembly): channel numbers, zapping,
 * groups, favorites and hidden dead channels. This file loads the playlist,
 * plays streams and turns remote-control keys into actions.
 */
'use strict';

var PLAYLIST_URL = 'https://iptv-org.github.io/iptv/index.m3u';
var STATE_KEY = 'iptv-state';
var ROW_H = 60;

var HELP = [
    'Up / Down, CH+ / CH−    change channel',
    '0–9                    go to channel number',
    'OK                     channel list',
    'Left / Right           previous / next group',
    'PRE-CH                 previous channel',
    'INFO                   channel info',
    'Red                    favorite      Green  favorites only',
    'Back                   close list / exit',
].join('\n');

var core = null;
var player = null;
var keyOf = null;
var health = null;
var names = [];
var urls = [];
var loaded = false;
var digitsPending = false;
var currentSeq = 0;
var currentTune = null;
var lastKey = 0;
var timers = { osd: 0, digit: 0 };
var checks = { total: 0, done: 0, changed: false, stop: null };
var panel = { open: false, items: new Uint32Array(0), favs: new Set(), current: -1, sel: 0, scroll: 0, rows: 0, els: [] };

function $(id) { return document.getElementById(id); }

/* --------------------------------------------------------------- startup */

function initCore() {
    var b64 = window.IPTV_WASM_B64;
    var bin = atob(b64);
    var bytes = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    wasm_bindgen.initSync({ module: bytes });
}

/** Playlist with conditional requests: downloads only when it changed. */
function loadPlaylist() {
    return Promise.all([Store.get('playlist'), Store.get('playlistMeta')]).catch(function () { return [null, null]; })
        .then(function (cached) {
            var text = cached[0];
            var meta = cached[1];
            var headers = {};
            if (text && meta && meta.url === PLAYLIST_URL) {
                if (meta.etag) headers['If-None-Match'] = meta.etag;
                if (meta.lastModified) headers['If-Modified-Since'] = meta.lastModified;
            }
            function download(h) {
                return fetch(PLAYLIST_URL, { headers: h, cache: 'no-store' }).then(function (r) {
                    if (r.status === 304 && text) return text;
                    if (!r.ok) throw new Error('HTTP ' + r.status);
                    return r.text().then(function (body) {
                        Store.set('playlist', body).catch(function () {});
                        Store.set('playlistMeta', {
                            url: PLAYLIST_URL,
                            etag: r.headers.get('ETag'),
                            lastModified: r.headers.get('Last-Modified'),
                        }).catch(function () {});
                        return body;
                    });
                });
            }
            return download(headers).catch(function (e) {
                // The conditional request can be refused where a plain one
                // works (cross-origin rules), so try a full download before
                // falling back to the saved copy (offline).
                return (Object.keys(headers).length ? download({}) : Promise.reject(e))
                    .catch(function (e2) {
                        if (text) return text;
                        throw e2;
                    });
            });
        });
}

/** One-time setup, then load. */
function boot() {
    player = createPlayer();
    keyOf = setupKeys();
    document.addEventListener('keydown', onKey);
    $('rows').addEventListener('click', onRowClick);
    document.addEventListener('visibilitychange', onVisibility);
    try {
        initCore();
    } catch (e) {
        showStatus('Could not start: ' + e);
        return;
    }
    load();
}

function load() {
    showStatus('Loading channels…');
    Promise.all([loadPlaylist(), HealthCache.load()]).then(function (res) {
        showStatus('Loading channels… (' + Math.round(res[0].length / 1024) + ' KB)');
        var state = '';
        try { state = localStorage.getItem(STATE_KEY) || ''; } catch (e) { /* ignore */ }
        core = new wasm_bindgen.WebCore(res[0], state);
        health = res[1];
        names = core.names();
        urls = core.urls();
        var jobs = applySavedChecks();
        loaded = true;
        hideStatus();
        perform(core.start());
        if (IS_TIZEN) runChecks(jobs);
        setInterval(tick, 1000);
    }).catch(function (e) {
        showStatus('Could not load the channel list.\n' + (e && e.message ? e.message : e) +
            '\n\nCheck the internet connection, then press OK to try again.');
        loaded = false;
        retryOnEnter = true;
    });
}

var retryOnEnter = false;

/** Hides channels already known to be dead; returns channels to (re)check, nearest first. */
function applySavedChecks() {
    var n = urls.length;
    var cur = core.current();
    var jobs = [];
    for (var i = 0; i < n; i++) {
        var h = health.get(urls[i]);
        if (h) {
            if (!h.ok) core.setDead(i, true);
            if (!h.fresh) jobs.push(i);
        } else {
            jobs.push(i);
        }
    }
    jobs.sort(function (a, b) { return ((a - cur + n) % n) - ((b - cur + n) % n); });
    return jobs;
}

function runChecks(jobs) {
    checks.total = jobs.length;
    checks.done = 0;
    checks.stop = startChecks(jobs.map(function (i) { return [i, urls[i]]; }), function (idx, url, verdict) {
        checks.done++;
        if (verdict === 'ok') {
            health.record(url, true);
            if (core.setDead(idx, false)) checks.changed = true;
        } else if (verdict === 'dead' && idx !== core.current()) {
            // The channel on screen is judged by the player, not the probe.
            health.record(url, false);
            if (core.setDead(idx, true)) checks.changed = true;
        }
    });
}

var lastHealthSave = Date.now();

function tick() {
    if (panel.open) {
        // Channels found dead while checking: refresh the list now and then,
        // never while the user is pressing keys.
        if (checks.changed && Date.now() - lastKey > 1500) {
            checks.changed = false;
            refreshList(true);
        } else {
            $('panel-status').textContent = checkStatus();
        }
    }
    if (Date.now() - lastHealthSave > 20000) {
        lastHealthSave = Date.now();
        health.save();
    }
}

function checkStatus() {
    return checks.done < checks.total
        ? 'Checking channels ' + Math.floor(checks.done * 100 / Math.max(1, checks.total)) + '%'
        : '';
}

/* ------------------------------------------------------ effects from core */

/** Carries out the core's instructions; returns true if a number is being typed. */
function perform(effectsJson) {
    var effects = JSON.parse(effectsJson);
    var typing = false;
    effects.forEach(function (e) {
        switch (e.type) {
            case 'tune': tune(e); break;
            case 'osd': showOsd(e.text, e.ms); break;
            case 'digitTimer':
                typing = true;
                clearTimeout(timers.digit);
                timers.digit = setTimeout(function () { tvAction('digit-timeout'); }, e.ms);
                break;
            case 'save':
                try { localStorage.setItem(STATE_KEY, core.state()); } catch (err) { /* ignore */ }
                break;
        }
    });
    if (!typing) clearTimeout(timers.digit);
    return typing;
}

function tvAction(name, arg) {
    if (!loaded) return;
    digitsPending = perform(core.action(name, arg === undefined ? null : arg));
    if (panel.open) refreshList(true);
}

function tune(e) {
    currentTune = e;
    currentSeq = e.seq;
    var seq = e.seq;
    player.play(e, function () {
        var idx = core.playbackStarted(seq);
        if (idx >= 0) health.record(urls[idx], true);
    }, function () {
        if (seq !== currentSeq) return;
        health.record(urls[core.current()], false);
        checks.changed = true;
        perform(core.playbackFailed(seq));
    });
    if (panel.open) {
        panel.current = e.idx;
        render();
    }
}

/* -------------------------------------------------------------- overlays */

function showOsd(text, ms) {
    var osd = $('osd');
    osd.textContent = text;
    osd.classList.toggle('right', panel.open);
    osd.classList.remove('hidden');
    clearTimeout(timers.osd);
    timers.osd = setTimeout(function () { osd.classList.add('hidden'); }, ms);
}

function showStatus(text) {
    var s = $('status');
    s.textContent = text;
    s.classList.remove('hidden');
}

function hideStatus() { $('status').classList.add('hidden'); }

/* ------------------------------------------------------------ channel list */

function lowerBound(arr, value) {
    var lo = 0, hi = arr.length;
    while (lo < hi) {
        var mid = (lo + hi) >>> 1;
        if (arr[mid] < value) lo = mid + 1; else hi = mid;
    }
    return Math.min(lo, Math.max(0, arr.length - 1));
}

function maxScroll() { return Math.max(0, panel.items.length - panel.rows); }

function openPanel() {
    if (!loaded) return;
    panel.open = true;
    $('panel').classList.remove('hidden');
    $('osd').classList.add('hidden');
    panel.rows = Math.max(1, Math.floor($('rows').clientHeight / ROW_H));
    buildRows();
    refreshList(false);
}

function closePanel() {
    panel.open = false;
    $('panel').classList.add('hidden');
}

function buildRows() {
    var rows = $('rows');
    if (panel.els.length === panel.rows) return;
    rows.innerHTML = '';
    panel.els = [];
    for (var r = 0; r < panel.rows; r++) {
        var el = document.createElement('div');
        el.className = 'row';
        el.style.top = (r * ROW_H) + 'px';
        el.dataset.row = r;
        el.innerHTML = '<span class="num"></span><span class="name"></span>';
        rows.appendChild(el);
        panel.els.push(el);
    }
}

/** Reloads the list; `keep` keeps the selection on the same screen row. */
function refreshList(keep) {
    var info = JSON.parse(core.listInfo());
    var oldSelIdx = panel.items.length ? panel.items[panel.sel] : -1;
    var oldRow = panel.sel - panel.scroll;
    panel.items = core.view();
    panel.favs = new Set(Array.from(core.favorites()));
    panel.current = info.current;
    $('panel-title').textContent = info.title;
    $('panel-status').textContent = checkStatus();
    if (keep && oldSelIdx >= 0) {
        panel.sel = lowerBound(panel.items, oldSelIdx);
        panel.scroll = Math.min(Math.max(0, panel.sel - oldRow), maxScroll());
    } else {
        panel.sel = lowerBound(panel.items, info.current);
        panel.scroll = Math.min(Math.max(0, panel.sel - Math.floor(panel.rows / 3)), maxScroll());
    }
    render();
}

function render() {
    for (var r = 0; r < panel.els.length; r++) {
        var el = panel.els[r];
        var i = panel.scroll + r;
        if (i >= panel.items.length) { el.style.visibility = 'hidden'; continue; }
        var idx = panel.items[i];
        var playing = idx === panel.current;
        el.style.visibility = 'visible';
        el.classList.toggle('selected', i === panel.sel);
        el.classList.toggle('playing', playing);
        el.firstChild.textContent = idx + 1;
        el.lastChild.textContent = (playing ? '▶ ' : '') + names[idx] + (panel.favs.has(idx) ? '  ★' : '');
    }
}

function move(delta) {
    if (!panel.items.length) return;
    panel.sel = Math.min(Math.max(0, panel.sel + delta), panel.items.length - 1);
    if (panel.sel < panel.scroll) panel.scroll = panel.sel;
    else if (panel.sel >= panel.scroll + panel.rows) panel.scroll = panel.sel - panel.rows + 1;
    render();
}

function selectedChannel() {
    return panel.items.length ? panel.items[panel.sel] : -1;
}

function onRowClick(ev) {
    var row = ev.target.closest('.row');
    if (!row) return;
    var i = panel.scroll + Number(row.dataset.row);
    if (i < panel.items.length) {
        panel.sel = i;
        perform(core.tuneTo(panel.items[i]));
    }
}

/* ------------------------------------------------------------------ input */

function onKey(ev) {
    var k = keyOf(ev);
    if (!k) return;
    ev.preventDefault();
    lastKey = Date.now();

    if (!loaded) {
        if (k.key === 'back') exitApp();
        else if (k.key === 'enter' && retryOnEnter) { retryOnEnter = false; load(); }
        return;
    }
    if (k.key === 'digit') { tvAction('digit', k.digit); return; }
    if (k.key === 'yellow') { showOsd(HELP, 8000); return; }

    if (panel.open) {
        switch (k.key) {
            case 'up': move(-1); break;
            case 'down': move(1); break;
            case 'chup': move(-panel.rows); break;
            case 'chdown': move(panel.rows); break;
            case 'left': tvAction('group-prev'); refreshList(false); break;
            case 'right': tvAction('group-next'); refreshList(false); break;
            case 'enter':
                if (digitsPending) tvAction('ok');
                else if (selectedChannel() >= 0) perform(core.tuneTo(selectedChannel()));
                break;
            case 'red':
                if (selectedChannel() >= 0) { perform(core.toggleFavorite(selectedChannel())); refreshList(true); }
                break;
            case 'green': tvAction('fav-only'); refreshList(false); break;
            case 'info': tvAction('info'); break;
            case 'prev': tvAction('prev'); break;
            case 'back': case 'list': closePanel(); break;
        }
        return;
    }

    switch (k.key) {
        case 'up': case 'chup': tvAction('up'); break;
        case 'down': case 'chdown': tvAction('down'); break;
        case 'left': tvAction('group-prev'); break;
        case 'right': tvAction('group-next'); break;
        case 'enter': if (digitsPending) tvAction('ok'); else openPanel(); break;
        case 'list': openPanel(); break;
        case 'info': tvAction('info'); break;
        case 'prev': tvAction('prev'); break;
        case 'red': tvAction('fav'); break;
        case 'green': tvAction('fav-only'); break;
        case 'back': exitApp(); break;
    }
}

function onVisibility() {
    if (!loaded) return;
    if (document.hidden) {
        player.stop();
        try { localStorage.setItem(STATE_KEY, core.state()); } catch (e) { /* ignore */ }
        health.save();
    } else if (currentTune) {
        tune(currentTune);
    }
}

// Problems are shown on screen: a TV has no console to look at.
window.addEventListener('error', function (e) {
    if (!loaded) showStatus('Error: ' + e.message + '\n' + (e.filename || '') + ':' + (e.lineno || ''));
});
window.addEventListener('unhandledrejection', function (e) {
    if (!loaded) showStatus('Error: ' + (e.reason && e.reason.message ? e.reason.message : e.reason));
});

window.addEventListener('load', boot);
