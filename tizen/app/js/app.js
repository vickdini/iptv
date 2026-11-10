/* IPTV for Samsung TVs. All channel logic lives in the shared Rust core
 * (crates/iptv-web, compiled to WebAssembly): channel numbers, zapping,
 * groups, favorites and hidden dead channels. This file loads the playlist,
 * plays streams and turns remote-control keys into actions.
 */
'use strict';

var PLAYLIST_URL = 'https://iptv-org.github.io/iptv/index.m3u';
var STATE_KEY = 'iptv-state';
var ROW_H = 60;

/* The screen. Built by the app itself (not index.html) so that updates
 * delivered by the loader can change it. */
var UI_HTML = [
    '<object id="av-player" type="application/avplayer"></object>',
    '<video id="html-player" autoplay playsinline></video>',
    '<div id="osd" class="osd hidden"></div>',
    '<div id="status" class="status">Loading channels…</div>',
    '<div id="panel" class="panel hidden">',
    '  <div class="panel-head">',
    '    <div id="panel-title" class="panel-title"></div>',
    '    <div id="panel-keys" class="panel-keys"></div>',
    '    <input id="search" class="search" type="text" placeholder="Search channels…" autocomplete="off" spellcheck="false">',
    '    <div id="panel-status" class="panel-status"></div>',
    '  </div>',
    '  <div id="rows" class="rows"></div>',
    '</div>',
].join('\n');

var HELP = [
    'Up / Down, CH+ / CH−    change channel',
    '0–9                    go to channel number',
    'OK                     channel list',
    'Left / Right           previous / next group',
    'PRE-CH                 previous channel',
    'INFO                   channel info',
    'Red                    favorite      Green  favorites only',
    'Blue                   search channels by name',
    'Yellow (in the list)   countries & languages',
    'Back                   close list / exit',
    '',
    'IPTV ' + (window.IPTV_VERSION || ''),
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

/** A playlist with conditional requests: downloads only when it changed. */
function loadPlaylist(url, key) {
    url = url || PLAYLIST_URL;
    key = key || 'playlist';
    return Promise.all([Store.get(key), Store.get(key + 'Meta')]).catch(function () { return [null, null]; })
        .then(function (cached) {
            var text = cached[0];
            var meta = cached[1];
            var headers = {};
            if (text && meta && meta.url === url) {
                if (meta.etag) headers['If-None-Match'] = meta.etag;
                if (meta.lastModified) headers['If-Modified-Since'] = meta.lastModified;
            }
            function download(h) {
                return fetch(url, { headers: h, cache: 'no-store' }).then(function (r) {
                    if (r.status === 304 && text) return text;
                    if (!r.ok) throw new Error('HTTP ' + r.status);
                    return r.text().then(function (body) {
                        Store.set(key, body).catch(function () {});
                        Store.set(key + 'Meta', {
                            url: url,
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

/** Builds the screen, then one-time setup, then load. */
function boot() {
    window.IPTV_BOOTED = true; // tells the loader this version started fine
    if (window.IPTV_STYLE && !document.getElementById('iptv-style')) {
        var style = document.createElement('style');
        style.id = 'iptv-style';
        style.textContent = window.IPTV_STYLE;
        document.head.appendChild(style);
    }
    if (!document.getElementById('osd')) document.body.innerHTML = UI_HTML;
    player = createPlayer();
    keyOf = setupKeys();
    document.addEventListener('keydown', onKey);
    $('rows').addEventListener('click', onRowClick);
    $('search').addEventListener('input', function () {
        if (!loaded) return;
        if (picker) { // the search box finds countries / languages while choosing them
            picker.query = $('search').value;
            pickerFilter();
            return;
        }
        core.setSearch($('search').value);
        refreshList(false, true);
    });
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
    // iptv-org also publishes the same channels grouped by country and by
    // language, for the countries & languages filter (optional).
    var companion = function (name, key) {
        return loadPlaylist(PLAYLIST_URL.replace(/index\.m3u$/, name), key).catch(function () { return ''; });
    };
    Promise.all([
        loadPlaylist(), HealthCache.load(),
        companion('index.country.m3u', 'byCountry'), companion('index.language.m3u', 'byLanguage'),
    ]).then(function (res) {
        showStatus('Loading channels… (' + Math.round(res[0].length / 1024) + ' KB)');
        var state = '';
        try { state = localStorage.getItem(STATE_KEY) || ''; } catch (e) { /* ignore */ }
        core = new wasm_bindgen.WebCore(res[0], state, res[2], res[3]);
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
    if (panel.open && !picker) {
        // Channels found dead while checking: refresh the list now and then,
        // never while the user is pressing keys.
        if (checks.changed && Date.now() - lastKey > 1500) {
            checks.changed = false;
            refreshList(true);
        } else {
            $('panel-status').textContent = listStatus();
        }
    }
    if (Date.now() - lastHealthSave > 20000) {
        lastHealthSave = Date.now();
        health.save();
    }
}

/** Under the list title: check progress, or the countries / languages shown. */
function listStatus() {
    var region = core.regionLabel();
    return checkStatus() || (region ? 'Showing: ' + region : '');
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
    $('search').value = core.searchText();
    refreshList(false);
}

/** Puts the cursor in the search box; the TV shows its on-screen keyboard. */
function startSearch() {
    if (!panel.open) openPanel();
    $('search').focus();
}

function endSearchTyping() {
    $('search').blur();
}

/** Group / favorites switching ends the search (the core does the same). */
function clearSearchBox() {
    $('search').value = '';
}

function closePanel() {
    endSearchTyping();
    if (picker) { picker = null; clearSearchBox(); }
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

/* ------------------------------------------------- countries & languages */

var KEYS_CHANNELS = '◀ ▶ group &nbsp; <span class="red">●</span> favorite &nbsp; <span class="green">●</span> favorites &nbsp; ' +
    '<span class="blue">●</span> search &nbsp; <span class="yellow">●</span> countries';
var KEYS_PICKER = '◀ ▶ countries / languages &nbsp; OK tick &nbsp; <span class="blue">●</span> find &nbsp; Back done';

/** The chooser, shown in the channel list's place: null when closed. */
var picker = null;

function fold(s) {
    return String(s).normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
}

function openPicker() {
    if (!panel.open) openPanel();
    picker = { tab: 'countries', query: '', info: null, all: [], items: [], sel: 0, scroll: 0 };
    clearSearchBox();
    pickerLoad(false);
}

function closePicker() {
    picker = null;
    endSearchTyping();
    clearSearchBox();
    refreshList(false);
}

/** Reloads names, counts and ticks; `keepSel` keeps the selection (after ticking). */
function pickerLoad(keepSel) {
    picker.info = JSON.parse(core.regionsInfo());
    picker.all = picker.info[picker.tab];
    pickerFilter(keepSel);
}

function pickerFilter(keepSel) {
    var q = fold(picker.query.trim());
    picker.items = [];
    for (var i = 0; i < picker.all.length; i++) {
        if (fold(picker.all[i][0]).indexOf(q) >= 0) picker.items.push(i);
    }
    if (!keepSel) { picker.sel = 0; picker.scroll = 0; }
    picker.sel = Math.min(picker.sel, Math.max(0, picker.items.length - 1));
    renderPicker();
}

function pickerChosen() {
    return picker.info[picker.tab === 'countries' ? 'chosenCountries' : 'chosenLanguages'];
}

function renderPicker() {
    var chosen = pickerChosen();
    var name = picker.tab === 'countries' ? 'Countries' : 'Languages';
    $('panel-title').textContent = chosen.length ? name + ' · ' + chosen.length + ' chosen' : name;
    $('panel-keys').innerHTML = KEYS_PICKER;
    var label = core.regionLabel();
    $('panel-status').textContent = !picker.all.length ? 'Not available for this playlist'
        : label ? 'Showing: ' + label : 'Nothing chosen: all channels';
    for (var r = 0; r < panel.els.length; r++) {
        var el = panel.els[r];
        var i = picker.scroll + r;
        if (i >= picker.items.length) { el.style.visibility = 'hidden'; continue; }
        var entry = picker.all[picker.items[i]];
        var on = chosen.indexOf(entry[0]) >= 0;
        el.style.visibility = 'visible';
        el.classList.toggle('selected', i === picker.sel);
        el.classList.toggle('playing', on);
        el.firstChild.textContent = entry[1];
        el.lastChild.textContent = (on ? '☑  ' : '☐  ') + entry[0];
    }
}

function pickerMove(delta) {
    if (!picker.items.length) return;
    picker.sel = Math.min(Math.max(0, picker.sel + delta), picker.items.length - 1);
    if (picker.sel < picker.scroll) picker.scroll = picker.sel;
    else if (picker.sel >= picker.scroll + panel.rows) picker.scroll = picker.sel - panel.rows + 1;
    renderPicker();
}

function pickerSwitchTab() {
    picker.tab = picker.tab === 'countries' ? 'languages' : 'countries';
    picker.query = '';
    clearSearchBox();
    pickerLoad(false);
}

/** Ticks / unticks the selected entry; applies at once. */
function pickerToggle() {
    if (!picker.items.length) return;
    var name = picker.all[picker.items[picker.sel]][0];
    var countries = picker.info.chosenCountries.slice();
    var languages = picker.info.chosenLanguages.slice();
    var list = picker.tab === 'countries' ? countries : languages;
    var at = list.indexOf(name);
    if (at >= 0) list.splice(at, 1); else list.push(name);
    perform(core.setRegions(JSON.stringify(countries), JSON.stringify(languages)));
    pickerLoad(true);
}

/**
 * Reloads the list; `keep` keeps the selection on the same screen row,
 * `toFirst` selects the first row (new search results).
 */
function refreshList(keep, toFirst) {
    if (picker) return renderPicker();
    $('panel-keys').innerHTML = KEYS_CHANNELS;
    var info = JSON.parse(core.listInfo());
    var oldSelIdx = panel.items.length ? panel.items[panel.sel] : -1;
    var oldRow = panel.sel - panel.scroll;
    panel.items = core.view();
    panel.favs = new Set(Array.from(core.favorites()));
    panel.current = info.current;
    $('panel-title').textContent = info.title;
    $('panel-status').textContent = listStatus();
    if (toFirst) {
        panel.sel = 0;
        panel.scroll = 0;
    } else if (keep && oldSelIdx >= 0) {
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
    // While the search box has the cursor, keys are typing (the on-screen
    // keyboard). Only leaving it is handled here.
    if (document.activeElement === $('search')) {
        var c = ev.keyCode;
        // Down / OK / Back, and the keyboard's Done (65376) / Cancel (65385).
        if (c === 40 || c === 13 || c === 10009 || c === 27 || c === 65376 || c === 65385) {
            ev.preventDefault();
            endSearchTyping();
        }
        return;
    }
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
    if (k.key === 'yellow') {
        // In the list: countries & languages. Otherwise: help.
        if (picker) closePicker();
        else if (panel.open) openPicker();
        else showOsd(HELP, 8000);
        return;
    }

    if (picker) {
        switch (k.key) {
            case 'up': if (picker.sel === 0) startSearch(); else pickerMove(-1); break;
            case 'down': pickerMove(1); break;
            case 'chup': pickerMove(-panel.rows); break;
            case 'chdown': pickerMove(panel.rows); break;
            case 'left': case 'right': pickerSwitchTab(); break;
            case 'enter': pickerToggle(); break;
            case 'blue': startSearch(); break;
            case 'back': case 'list': closePicker(); break;
        }
        return;
    }

    if (panel.open) {
        switch (k.key) {
            case 'up': if (panel.sel === 0) startSearch(); else move(-1); break;
            case 'down': move(1); break;
            case 'blue': startSearch(); break;
            case 'chup': move(-panel.rows); break;
            case 'chdown': move(panel.rows); break;
            case 'left': clearSearchBox(); tvAction('group-prev'); refreshList(false); break;
            case 'right': clearSearchBox(); tvAction('group-next'); refreshList(false); break;
            case 'enter':
                if (digitsPending) tvAction('ok');
                else if (selectedChannel() >= 0) perform(core.tuneTo(selectedChannel()));
                break;
            case 'red':
                if (selectedChannel() >= 0) { perform(core.toggleFavorite(selectedChannel())); refreshList(true); }
                break;
            case 'green': clearSearchBox(); tvAction('fav-only'); refreshList(false); break;
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
        case 'blue': startSearch(); break;
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

// The loader announces a downloaded update before restarting into it.
window.addEventListener('iptv-update', function (e) {
    showOsd('IPTV ' + e.detail + ' installed.\nRestarting…', 4000);
});

// The loader may run this after the page's load event has already fired.
if (document.readyState === 'complete') boot();
else window.addEventListener('load', boot);
