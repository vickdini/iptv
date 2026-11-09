/* IPTV loader for Samsung TVs: keeps the app up to date automatically.
 *
 * Tizen doesn't let an app reinstall itself, so the installed package is a
 * small, permanent loader plus the app bundle it was built with. On every
 * start the loader:
 *   1. runs the newest bundle it has (shipped, or a newer one downloaded
 *      earlier), so starting never waits for the network;
 *   2. a few seconds later asks GitHub Pages for the current version, and if
 *      it is newer downloads the bundle, keeps it, and restarts into it.
 * If a downloaded bundle fails to start, it is discarded and the shipped one
 * is used again.
 */
(function () {
    'use strict';

    var SHIPPED_VERSION = '@VERSION@';
    // ?updateBase=<url> overrides it for testing (the TV never passes one).
    var override = /[?&]updateBase=([^&]+)/.exec(location.search);
    var UPDATE_BASE = override ? decodeURIComponent(override[1]) : 'https://vickdini.github.io/iptv/tv/';
    var CHECK_DELAY_MS = 8000;

    function parse(v) {
        var m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(String(v || '').trim());
        return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : null;
    }
    function newer(a, b) { // a > b
        var x = parse(a), y = parse(b);
        if (!x) return false;
        if (!y) return true;
        for (var i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
        return false;
    }

    // Own database, separate from the app's, so app changes can't break it.
    function db() {
        return new Promise(function (resolve, reject) {
            var req = indexedDB.open('iptv-loader', 1);
            req.onupgradeneeded = function () { req.result.createObjectStore('kv'); };
            req.onsuccess = function () { resolve(req.result); };
            req.onerror = function () { reject(req.error); };
        });
    }
    function store(mode, fn) {
        return db().then(function (d) {
            return new Promise(function (resolve, reject) {
                var t = d.transaction('kv', mode);
                var req = fn(t.objectStore('kv'));
                t.oncomplete = function () { resolve(req && req.result); };
                t.onerror = function () { reject(t.error); };
            });
        });
    }
    var getCached = function () { return store('readonly', function (s) { return s.get('bundle'); }).catch(function () { return null; }); };
    var putCached = function (b) { return store('readwrite', function (s) { return s.put(b, 'bundle'); }); };
    var dropCached = function () { return store('readwrite', function (s) { return s.delete('bundle'); }).catch(function () {}); };

    function runShipped() {
        window.IPTV_VERSION = SHIPPED_VERSION;
        var s = document.createElement('script');
        s.src = 'js/bundle.js';
        document.body.appendChild(s);
    }

    function runCode(code, version) {
        window.IPTV_VERSION = version;
        var s = document.createElement('script');
        s.textContent = code;
        document.body.appendChild(s);
    }

    function checkForUpdate() {
        fetch(UPDATE_BASE + 'version.json', { cache: 'no-store' })
            .then(function (r) { return r.ok ? r.json() : null; })
            .then(function (info) {
                if (!info || !newer(info.version, window.IPTV_VERSION)) return null;
                return fetch(UPDATE_BASE + 'bundle.js?v=' + encodeURIComponent(info.version), { cache: 'no-store' })
                    .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.text(); })
                    .then(function (code) {
                        // The bundle ends with this marker: a cut-off download is never used.
                        if (code.indexOf('/*IPTV-BUNDLE-END*/') < 0) throw new Error('incomplete bundle');
                        return putCached({ version: info.version, code: code });
                    })
                    .then(function () {
                        window.dispatchEvent(new CustomEvent('iptv-update', { detail: info.version }));
                        setTimeout(function () { location.reload(); }, 3500);
                    });
            })
            .catch(function () { /* offline or no update: try again next start */ });
    }

    getCached().then(function (cached) {
        if (cached && cached.code && newer(cached.version, SHIPPED_VERSION)) {
            runCode(cached.code, cached.version);
            // A broken update would never start the app: go back to the shipped one.
            setTimeout(function () {
                if (!window.IPTV_BOOTED) dropCached().then(function () { location.reload(); });
            }, 5000);
        } else {
            if (cached) dropCached(); // the install is newer than the download
            runShipped();
        }
        setTimeout(checkForUpdate, CHECK_DELAY_MS);
    });
})();
