/* Background channel checks, as in the desktop and Android apps:
 * every channel is probed with a quick HTTP request; HLS playlists must be
 * valid and, for master playlists, the first variant must be reachable.
 * Results are saved for 6 hours so later launches hide dead channels at once.
 *
 * Only runs inside the TV app: Samsung's runtime lets a packaged app read
 * responses from any server (<access origin="*">); a normal browser would
 * block that (CORS), making every channel look dead.
 */
'use strict';

var CHECK_TTL_MS = 6 * 3600 * 1000;
var CHECK_WORKERS = 6;
var CHECK_TIMEOUT_MS = 10000;
var MAX_PLAYLIST_CHARS = 512 * 1024;

/** Saved results: { url: { ok: bool, at: ms } }. */
function HealthCache(entries) {
    this.entries = entries || {};
    this.dirty = false;
}
HealthCache.load = function () {
    return Store.get('health').then(function (e) { return new HealthCache(e); }, function () { return new HealthCache(); });
};
HealthCache.prototype.get = function (url) {
    var e = this.entries[url];
    return e ? { ok: e.ok, fresh: Date.now() - e.at < CHECK_TTL_MS } : null;
};
HealthCache.prototype.record = function (url, ok) {
    this.entries[url] = { ok: ok, at: Date.now() };
    this.dirty = true;
};
HealthCache.prototype.save = function () {
    if (!this.dirty) return Promise.resolve();
    this.dirty = false;
    return Store.set('health', this.entries).catch(function () { /* best effort */ });
};

function fetchWithTimeout(url) {
    var ctrl = new AbortController();
    var timer = setTimeout(function () { ctrl.abort(); }, CHECK_TIMEOUT_MS);
    return fetch(url, { signal: ctrl.signal, cache: 'no-store', credentials: 'omit' })
        .then(function (r) { r.timer = timer; return r; }, function (e) { clearTimeout(timer); throw e; });
}

function readText(resp, limit) {
    return resp.text().then(function (t) { clearTimeout(resp.timer); return t.slice(0, limit); },
        function (e) { clearTimeout(resp.timer); throw e; });
}

function firstVariant(playlist) {
    var lines = playlist.split(/\r?\n/);
    for (var i = 0; i < lines.length; i++) {
        if (lines[i].indexOf('#EXT-X-STREAM-INF') === 0) {
            for (var j = i + 1; j < lines.length; j++) {
                var l = lines[j].trim();
                if (l && l[0] !== '#') return l;
            }
        }
    }
    return null;
}

/** Resolves to 'ok', 'dead' or 'unknown'. */
function probe(url) {
    if (!/^https?:\/\//i.test(url)) return Promise.resolve('unknown'); // rtmp:// etc.
    return fetchWithTimeout(url).then(function (r) {
        if (!r.ok) { clearTimeout(r.timer); return 'dead'; }
        var finalUrl = r.url || url;
        var hls = /\.m3u8/i.test(finalUrl) || /mpegurl/i.test(r.headers.get('content-type') || '');
        if (!hls) {
            clearTimeout(r.timer);
            if (r.body && r.body.cancel) r.body.cancel();
            return 'ok';
        }
        return readText(r, MAX_PLAYLIST_CHARS).then(function (text) {
            if (text.replace(/^\s+/, '').indexOf('#EXTM3U') !== 0) return 'dead';
            var variant = firstVariant(text);
            if (!variant) return text.indexOf('#EXTINF') >= 0 ? 'ok' : 'dead';
            var next;
            try { next = new URL(variant, finalUrl).href; } catch (e) { return 'dead'; }
            return fetchWithTimeout(next).then(function (r2) {
                if (!r2.ok) { clearTimeout(r2.timer); return 'dead'; }
                return readText(r2, MAX_PLAYLIST_CHARS).then(function (t2) {
                    return t2.replace(/^\s+/, '').indexOf('#EXTM3U') === 0 ? 'ok' : 'dead';
                });
            });
        });
    }).catch(function () {
        return 'dead'; // timeout, DNS failure, connection refused...
    });
}

/**
 * Checks `jobs` (array of [idx, url]) in order with a few parallel workers,
 * calling onResult(idx, url, verdict). Returns a function that stops it.
 */
function startChecks(jobs, onResult) {
    var next = 0;
    var stopped = false;
    function worker() {
        if (stopped || next >= jobs.length) return Promise.resolve();
        var job = jobs[next++];
        return probe(job[1]).then(function (verdict) {
            if (!stopped) onResult(job[0], job[1], verdict);
        }).then(worker);
    }
    for (var i = 0; i < CHECK_WORKERS; i++) worker();
    return function () { stopped = true; };
}
