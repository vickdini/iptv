/* Platform layer: video player, remote keys and storage.
 *
 * On a Samsung TV this uses AVPlay (the TV's hardware video player) and the
 * Tizen remote-control API. In a desktop browser (for development) it falls
 * back to an HTML <video> element, using hls.js for HLS streams if present.
 */
'use strict';

var IS_TIZEN = typeof window.tizen !== 'undefined' && typeof window.webapis !== 'undefined' &&
    typeof window.webapis.avplay !== 'undefined';

/* ---------------------------------------------------------------- storage */

/** Tiny key/value store on IndexedDB (localStorage is too small for the playlist). */
var Store = (function () {
    var dbPromise = null;

    function db() {
        if (!dbPromise) {
            dbPromise = new Promise(function (resolve, reject) {
                var req = indexedDB.open('iptv', 1);
                req.onupgradeneeded = function () { req.result.createObjectStore('kv'); };
                req.onsuccess = function () { resolve(req.result); };
                req.onerror = function () { reject(req.error); };
            });
        }
        return dbPromise;
    }

    function tx(mode, fn) {
        return db().then(function (d) {
            return new Promise(function (resolve, reject) {
                var t = d.transaction('kv', mode);
                var req = fn(t.objectStore('kv'));
                t.oncomplete = function () { resolve(req && req.result); };
                t.onerror = function () { reject(t.error); };
            });
        });
    }

    return {
        get: function (key) { return tx('readonly', function (s) { return s.get(key); }); },
        set: function (key, value) { return tx('readwrite', function (s) { return s.put(value, key); }); },
    };
})();

/* ----------------------------------------------------------------- player */

/**
 * Plays one stream at a time. `play(tune, onReady, onError)`: callbacks for
 * an old stream are ignored once a newer one was started.
 */
function TizenPlayer() {
    var token = 0;
    var av = webapis.avplay;

    function stopQuietly() {
        try { av.stop(); } catch (e) { /* not playing */ }
        try { av.close(); } catch (e) { /* not open */ }
    }

    this.play = function (tune, onReady, onError) {
        var my = ++token;
        var ready = false;
        var mine = function () { return my === token; };
        stopQuietly();
        try {
            av.open(tune.url);
            av.setDisplayRect(0, 0, 1920, 1080);
            av.setDisplayMethod('PLAYER_DISPLAY_MODE_LETTER_BOX');
            try { av.setStreamingProperty('USER_AGENT', tune.userAgent); } catch (e) { /* older TVs */ }
            av.setListener({
                onbufferingcomplete: function () {
                    if (mine() && !ready) { ready = true; onReady(); }
                },
                onstreamcompleted: function () { if (mine()) onError('stream ended'); },
                onerror: function (e) { if (mine()) onError(String(e)); },
            });
            av.prepareAsync(function () {
                if (!mine()) return;
                av.play();
                if (!ready) { ready = true; onReady(); }
            }, function (e) {
                if (mine()) onError(e && e.name ? e.name : String(e));
            });
        } catch (e) {
            var msg = e && e.name ? e.name : String(e);
            setTimeout(function () { if (mine()) onError(msg); }, 0);
        }
    };

    this.stop = function () { token++; stopQuietly(); };
}

/** Desktop-browser fallback for development. */
function HtmlPlayer() {
    var video = document.getElementById('html-player');
    var hls = null;
    var token = 0;
    video.style.display = 'block';

    this.play = function (tune, onReady, onError) {
        var my = ++token;
        var mine = function () { return my === token; };
        if (hls) { hls.destroy(); hls = null; }
        video.onplaying = function () { if (mine()) onReady(); };
        video.onerror = function () { if (mine()) onError('media error'); };
        if (window.Hls && window.Hls.isSupported() && /\.m3u8/i.test(tune.url)) {
            hls = new window.Hls();
            hls.on(window.Hls.Events.ERROR, function (_, data) { if (data.fatal && mine()) onError(data.details); });
            hls.loadSource(tune.url);
            hls.attachMedia(video);
        } else {
            video.src = tune.url;
        }
        var p = video.play();
        if (p && p.catch) p.catch(function () { /* autoplay policy; onplaying fires later */ });
    };

    this.stop = function () {
        token++;
        if (hls) { hls.destroy(); hls = null; }
        video.removeAttribute('src');
        video.load();
    };
}

function createPlayer() {
    return IS_TIZEN ? new TizenPlayer() : new HtmlPlayer();
}

/* ------------------------------------------------------------------- keys */

/**
 * Returns a function mapping a KeyboardEvent to a key name: 'up', 'down',
 * 'left', 'right', 'enter', 'back', 'digit' (with .digit), 'chup', 'chdown',
 * 'info', 'prev', 'list', 'red', 'green', 'yellow', or null.
 */
function setupKeys() {
    var codes = {};
    var tizenKeys = ['ChannelUp', 'ChannelDown', 'Info', 'PreviousChannel', 'ChannelList',
        'ColorF0Red', 'ColorF1Green', 'ColorF2Yellow', 'ColorF3Blue',
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];
    if (IS_TIZEN) {
        tizenKeys.forEach(function (name) {
            try {
                tizen.tvinputdevice.registerKey(name);
                codes[tizen.tvinputdevice.getKey(name).code] = name;
            } catch (e) { /* key not on this TV */ }
        });
    }

    return function (ev) {
        var k = ev.keyCode;
        var name = codes[k];
        if (name && /^[0-9]$/.test(name)) return { key: 'digit', digit: name };
        switch (name) {
            case 'ChannelUp': return { key: 'chup' };
            case 'ChannelDown': return { key: 'chdown' };
            case 'Info': return { key: 'info' };
            case 'PreviousChannel': return { key: 'prev' };
            case 'ChannelList': return { key: 'list' };
            case 'ColorF0Red': return { key: 'red' };
            case 'ColorF1Green': return { key: 'green' };
            case 'ColorF2Yellow': return { key: 'yellow' };
            case 'ColorF3Blue': return { key: 'blue' };
        }
        if (k >= 48 && k <= 57) return { key: 'digit', digit: String(k - 48) };
        if (k >= 96 && k <= 105) return { key: 'digit', digit: String(k - 96) };
        switch (k) {
            case 37: return { key: 'left' };
            case 38: return { key: 'up' };
            case 39: return { key: 'right' };
            case 40: return { key: 'down' };
            case 13: return { key: 'enter' };
            case 10009: case 27: return { key: 'back' };     // remote Back / Esc
            case 33: return { key: 'chup' };                  // PageUp
            case 34: return { key: 'chdown' };                // PageDown
            case 8: return { key: 'prev' };                   // Backspace
            case 73: return { key: 'info' };                  // I
            case 76: return { key: 'list' };                  // L
            case 191: return { key: 'blue' };                 // "/" = search
            case 83: return { key: 'red' };                   // S
        }
        return null;
    };
}

function exitApp() {
    if (IS_TIZEN) {
        try { tizen.application.getCurrentApplication().exit(); } catch (e) { /* ignore */ }
    }
}
