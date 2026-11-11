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
    '<div id="subs" class="subs hidden"><span id="subs-text"></span></div>',
    '<div id="osd" class="osd hidden"></div>',
    '<div id="tracks" class="tracks hidden"></div>',
    '<div id="status" class="status">' + T('Loading channels…') + '</div>',
    '<div id="panel" class="panel hidden">',
    '  <div class="panel-head">',
    '    <div id="panel-title" class="panel-title"></div>',
    '    <div id="panel-keys" class="panel-keys"></div>',
    '    <input id="search" class="search" type="text" placeholder="' + T('Search channels…') + '" autocomplete="off" spellcheck="false">',
    '    <div id="panel-status" class="panel-status"></div>',
    '  </div>',
    '  <div id="rows" class="rows"></div>',
    '</div>',
].join('\n');

var core = null;
var player = null;
var keyOf = null;
var health = null;
var names = [];
var logos = [];
var urls = [];
var loaded = false;
var digitsPending = false;
var currentSeq = 0;
var currentTune = null;
var lastKey = 0;
var timers = { osd: 0, digit: 0, subs: 0, tracks: 0 };
/** The audio & subtitles menu while open: `{row}` (0 audio, 1 subtitles). */
var tracksMenu = null;
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
    player.onSubtitle = showSubtitle;
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
        showStatus(TF('Could not start: {0}', e));
        return;
    }
    load();
}

function load() {
    showStatus(T('Loading channels…'));
    // iptv-org also publishes the same channels grouped by country and by
    // language, for the countries & languages filter (optional).
    var companion = function (name, key) {
        return loadPlaylist(PLAYLIST_URL.replace(/index\.m3u$/, name), key).catch(function () { return ''; });
    };
    Promise.all([
        loadPlaylist(), HealthCache.load(),
        companion('index.country.m3u', 'byCountry'), companion('index.language.m3u', 'byLanguage'),
    ]).then(function (res) {
        showStatus(TF('Loading channels… ({0} KB)', Math.round(res[0].length / 1024)));
        var state = '';
        try { state = localStorage.getItem(STATE_KEY) || ''; } catch (e) { /* ignore */ }
        core = new wasm_bindgen.WebCore(res[0], state, res[2], res[3]);
        core.setLanguage(navigator.language || 'en');
        health = res[1];
        names = core.names();
        logos = core.logos();
        urls = core.urls();
        var jobs = applySavedChecks();
        loaded = true;
        hideStatus();
        perform(core.start());
        if (IS_TIZEN) runChecks(jobs);
        setInterval(tick, 1000);
    }).catch(function (e) {
        showStatus(T('Could not load the channel list.') + '\n' + (e && e.message ? e.message : e) +
            '\n\n' + T('Check the internet connection, then press OK to try again.'));
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
    return checkStatus() || (region ? TF('Showing: {0}', region) : '');
}

function checkStatus() {
    return checks.done < checks.total
        ? TF('Checking channels {0}%', Math.floor(checks.done * 100 / Math.max(1, checks.total)))
        : '';
}

/* ------------------------------------------------------ effects from core */

/** Carries out the core's instructions; returns true if a number is being typed. */
function perform(effectsJson) {
    var effects = JSON.parse(effectsJson);
    var typing = false;
    var tuned = -1; // the banner after a tune gets the channel's logo
    effects.forEach(function (e) {
        switch (e.type) {
            case 'tune': tune(e); tuned = e.idx; break;
            case 'osd': showOsd(e.text, e.ms, tuned); break;
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
    closeTracks();
    hideSubtitle();
    player.play(e, function () {
        var idx = core.playbackStarted(seq);
        if (idx >= 0) health.record(urls[idx], true);
        // Track lists fill in once the stream runs.
        setTimeout(function () { if (seq === currentSeq) onTracks(seq); }, 1500);
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

function showOsd(text, ms, logoOf) {
    var osd = $('osd');
    osd.textContent = '';
    var url = logoOf >= 0 ? logos[logoOf] : '';
    if (url) {
        var img = document.createElement('img');
        img.className = 'osd-logo';
        img.referrerPolicy = 'no-referrer';
        img.alt = '';
        img.onerror = function () { img.remove(); };
        img.src = url;
        osd.appendChild(img);
    }
    var span = document.createElement('span');
    span.textContent = text;
    osd.appendChild(span);
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

/** Where `value` is in the list, else (lists sorted by number) the next one. */
function positionOf(arr, value) {
    var i = Array.prototype.indexOf.call(arr, value);
    return i >= 0 ? i : lowerBound(arr, value);
}

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
        // No referrer: image hosts such as imgur refuse other sites' pages.
        el.innerHTML = '<span class="num"></span><img class="logo" alt="" referrerpolicy="no-referrer"><span class="name"></span>';
        el.querySelector('.logo').onerror = function () { this.style.visibility = 'hidden'; };
        rows.appendChild(el);
        panel.els.push(el);
    }
}

/* ------------------------------------------------- countries & languages */

var KEYS_CHANNELS = '◀ ▶ ' + T('recent, groups') + ' &nbsp; <span class="red">●</span> ' + T('favorite') +
    ' &nbsp; <span class="green">●</span> ' + T('favorites') + ' &nbsp; <span class="blue">●</span> ' + T('search') +
    ' &nbsp; <span class="yellow">●</span> ' + T('countries');
var KEYS_PICKER = '◀ ▶ ' + T('countries / languages') + ' &nbsp; ' + T('OK tick') + ' &nbsp; <span class="red">●</span> ' +
    T('clear all') + ' &nbsp; <span class="blue">●</span> ' + T('find') + ' &nbsp; ' + T('Back done');

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
    // While anything is chosen, the first row clears every filter (-1).
    var chosenAny = picker.info.chosenCountries.length || picker.info.chosenLanguages.length;
    if (chosenAny && !q) picker.items.push(-1);
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
    var name = picker.tab === 'countries' ? T('Countries') : T('Languages');
    $('panel-title').textContent = chosen.length ? name + ' · ' + chosen.length + ' chosen' : name;
    $('panel-keys').innerHTML = KEYS_PICKER;
    var label = core.regionLabel();
    $('panel-status').textContent = !picker.all.length ? T('Not available for this playlist')
        : label ? TF('Showing: {0}', label) : T('Nothing chosen: all channels');
    for (var r = 0; r < panel.els.length; r++) {
        var el = panel.els[r];
        var i = picker.scroll + r;
        setLogo(el.querySelector('.logo'), ''); // no channel logos in this list
        if (i >= picker.items.length) { el.style.display = 'none'; continue; }
        el.style.display = '';
        if (picker.items[i] === -1) {
            el.style.visibility = 'visible';
            el.classList.toggle('selected', i === picker.sel);
            el.classList.add('playing');
            el.firstChild.textContent = '';
            el.lastChild.textContent = T('✕  Clear all filters (show every channel)');
            continue;
        }
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
    if (picker.items[picker.sel] === -1) return pickerClearAll();
    var name = picker.all[picker.items[picker.sel]][0];
    var countries = picker.info.chosenCountries.slice();
    var languages = picker.info.chosenLanguages.slice();
    var list = picker.tab === 'countries' ? countries : languages;
    var at = list.indexOf(name);
    if (at >= 0) list.splice(at, 1); else list.push(name);
    perform(core.setRegions(JSON.stringify(countries), JSON.stringify(languages)));
    pickerLoad(true);
}

/** Clears every country and language: all channels again. */
function pickerClearAll() {
    perform(core.setRegions('[]', '[]'));
    pickerLoad(false);
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
        panel.sel = positionOf(panel.items, oldSelIdx);
        panel.scroll = Math.min(Math.max(0, panel.sel - oldRow), maxScroll());
    } else {
        panel.sel = positionOf(panel.items, info.current);
        panel.scroll = Math.min(Math.max(0, panel.sel - Math.floor(panel.rows / 3)), maxScroll());
    }
    render();
}

function render() {
    for (var r = 0; r < panel.els.length; r++) {
        var el = panel.els[r];
        var i = panel.scroll + r;
        // display, not visibility: a logo set visible would still show.
        if (i >= panel.items.length) { el.style.display = 'none'; continue; }
        var idx = panel.items[i];
        var playing = idx === panel.current;
        el.style.display = '';
        el.classList.toggle('selected', i === panel.sel);
        el.classList.toggle('playing', playing);
        el.firstChild.textContent = idx + 1;
        setLogo(el.querySelector('.logo'), logos[idx]);
        el.lastChild.textContent = (playing ? '▶ ' : '') + names[idx] + (panel.favs.has(idx) ? '  ★' : '');
    }
}

/** Shows a channel logo in an <img> (hidden when there is none / it fails). */
function setLogo(img, url) {
    if (!url) { img.removeAttribute('src'); img.style.visibility = 'hidden'; return; }
    if (img.getAttribute('src') !== url) {
        img.style.visibility = 'visible';
        img.src = url;
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

/* ------------------------------------------- audio language & subtitles */

function trackLabel(t, i) {
    if (t.label) return t.label;
    if (t.lang) return core.languageName(t.lang);
    return TF('Track {0}', i + 1);
}

function prefs() { return JSON.parse(core.trackPrefs()); }

/** The stream's tracks are known: use the remembered languages, and say
 *  when there is a choice. */
function onTracks(seq) {
    var t = player.tracks();
    var p = prefs();
    if (p.audio) {
        var a = t.audio.filter(function (x) { return core.sameLanguage(x.lang, p.audio); })[0];
        if (a && !a.selected) player.select('audio', a.id);
    }
    if (p.subtitles) {
        var s = t.subs.filter(function (x) { return core.sameLanguage(x.lang, p.subtitles); })[0];
        if (s) player.select('subs', s.id);
    }
    var parts = [];
    if (t.audio.length > 1) parts.push(TF('{0} audio languages', t.audio.length));
    if (t.subs.length) parts.push(TF('{0} subtitles', t.subs.length));
    if (parts.length && !panel.open && seq === currentSeq) {
        showOsd(TF('This channel has {0}\nYellow: choose', parts.join(' · ')), 5000);
    }
}

function showSubtitle(text, ms) {
    // Drop markup such as <i> or {\an8}.
    var clean = text.replace(/<[^>]*>/g, '').replace(/\{[^}]*\}/g, '').trim();
    if (!clean) { hideSubtitle(); return; }
    $('subs-text').textContent = clean;
    $('subs').classList.remove('hidden');
    clearTimeout(timers.subs);
    timers.subs = setTimeout(hideSubtitle, ms > 0 ? ms : 4000);
}

function hideSubtitle() {
    clearTimeout(timers.subs);
    $('subs').classList.add('hidden');
}

/** Yellow outside the list: the menu, or help when there is no choice. */
function openTracks() {
    var t = player.tracks();
    if (t.audio.length < 2 && !t.subs.length) { showOsd(helpText(), 8000); return; }
    tracksMenu = { row: t.audio.length > 1 ? 0 : 1 };
    $('osd').classList.add('hidden');
    renderTracks();
}

function closeTracks() {
    tracksMenu = null;
    clearTimeout(timers.tracks);
    $('tracks').classList.add('hidden');
}

function renderTracks() {
    var t = player.tracks();
    var html = '<div class="title">' + escapeHtml(T('Audio & subtitles')) + ' &nbsp; ▲▼ ◀▶ &nbsp; ' +
        escapeHtml(T('Back: close')) + '</div>';
    function row(n, name, value) {
        html += '<div class="item' + (tracksMenu.row === n ? ' selected' : '') + '"><span>' + name +
            '</span><span class="value">◀ ' + escapeHtml(value) + ' ▶</span></div>';
    }
    function current(list, off) {
        for (var i = 0; i < list.length; i++) {
            if (list[i].selected) return trackLabel(list[i], i) + '  (' + (i + 1) + '/' + list.length + ')';
        }
        return off;
    }
    if (t.audio.length > 1) row(0, escapeHtml(T('Audio')), current(t.audio, '—'));
    if (t.subs.length) row(1, escapeHtml(T('Subtitles')), current(t.subs, T('Off')));
    var el = $('tracks');
    el.innerHTML = html;
    el.classList.remove('hidden');
    clearTimeout(timers.tracks);
    timers.tracks = setTimeout(closeTracks, 10000);
}

function escapeHtml(s) {
    return String(s).replace(/[&<>"]/g, function (c) {
        return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c];
    });
}

/** Next (delta 1) or previous audio track / subtitles (with "off"); remembers
 *  the language. Returns the new label, or null if there is no choice. */
function stepTrack(kind, delta) {
    var t = player.tracks();
    var list = kind === 'audio' ? t.audio : t.subs;
    if (kind === 'audio' ? list.length < 2 : !list.length) return null;
    var pos = -1;
    for (var i = 0; i < list.length; i++) if (list[i].selected) pos = i;
    var p = prefs();
    var label;
    if (kind === 'audio') {
        var a = ((pos < 0 ? 0 : pos) + delta + list.length) % list.length;
        player.select('audio', list[a].id);
        perform(core.setTrackPrefs(list[a].lang || p.audio, p.subtitles));
        label = trackLabel(list[a], a) + '  (' + (a + 1) + '/' + list.length + ')';
    } else {
        // Positions: -1 = off, then each track.
        var n = list.length + 1;
        var s = ((pos + 1 + delta + n) % n) - 1;
        if (s < 0) {
            player.select('subs', null);
            hideSubtitle();
            perform(core.setTrackPrefs(p.audio, null));
            label = T('Off');
        } else {
            player.select('subs', list[s].id);
            perform(core.setTrackPrefs(p.audio, list[s].lang || null));
            label = trackLabel(list[s], s) + '  (' + (s + 1) + '/' + list.length + ')';
        }
    }
    return label;
}

/** Audio / subtitle remote keys: the next one, shown briefly. */
function quickTrack(kind) {
    var label = stepTrack(kind, 1);
    if (label === null) {
        showOsd(T(kind === 'audio' ? 'This channel has a single audio language' : 'This channel has no subtitles'), 2500);
    } else {
        showOsd(TF(kind === 'audio' ? 'Audio: {0}' : 'Subtitles: {0}', label), 2500);
    }
}

function tracksKey(key) {
    var t = player.tracks();
    var rows = [];
    if (t.audio.length > 1) rows.push(0);
    if (t.subs.length) rows.push(1);
    switch (key) {
        case 'up': case 'down':
            if (rows.length > 1) tracksMenu.row = tracksMenu.row === 0 ? 1 : 0;
            break;
        case 'left': case 'right': case 'enter':
            stepTrack(tracksMenu.row === 0 ? 'audio' : 'subs', key === 'left' ? -1 : 1);
            break;
        default:
            closeTracks();
            return;
    }
    // The player takes a moment to switch; show the result after it.
    renderTracks();
    setTimeout(function () { if (tracksMenu) renderTracks(); }, 400);
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
    if (k.key === 'digit') { closeTracks(); tvAction('digit', k.digit); return; }
    if (k.key === 'audio' || k.key === 'subtitles') {
        quickTrack(k.key === 'audio' ? 'audio' : 'subs');
        if (tracksMenu) renderTracks();
        return;
    }
    if (tracksMenu) {
        if (k.key === 'yellow' || k.key === 'back') { closeTracks(); return; }
        if (['up', 'down', 'left', 'right', 'enter'].indexOf(k.key) >= 0) { tracksKey(k.key); return; }
        closeTracks();
    }
    if (k.key === 'yellow') {
        // In the list: countries & languages. Otherwise: audio & subtitles.
        if (picker) closePicker();
        else if (panel.open) openPicker();
        else openTracks();
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
            case 'red': pickerClearAll(); break;
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
    showOsd(TF('IPTV {0} installed.\nRestarting…', e.detail), 4000);
});

// The loader may run this after the page's load event has already fired.
if (document.readyState === 'complete') boot();
else window.addEventListener('load', boot);
