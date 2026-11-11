package io.github.vickdini.iptv

/**
 * The shared Rust core (crates/iptv-android): TV tuner, playlist updates and
 * channel checks — the same code the desktop app uses.
 *
 * Calls that return effects give a JSON array of instructions for the UI:
 * `{"type":"tune","seq",..,"url","userAgent","referrer","name","number","idx"}`,
 * `{"type":"osd","text","ms"}`, `{"type":"digitTimer","ms"}`.
 * Errors are thrown as RuntimeException.
 */
object Core {
    init {
        System.loadLibrary("iptv_android")
    }

    /** Must be called first: where to keep the playlist copy / check results and settings. */
    @JvmStatic external fun init(cacheDir: String, configDir: String): String

    /** Downloads (or confirms up to date) the playlist and starts channel checks. Blocking. */
    @JvmStatic external fun loadPlaylist(url: String): String

    /** Effects for startup: tune the last watched channel. */
    @JvmStatic external fun start(): String

    /** Remote-control action ("up", "down", "digit" + "0".."9", "ok", "digit-timeout",
     *  "prev", "info", "group-next", "group-prev", "fav", "fav-only", "help"). */
    @JvmStatic external fun action(name: String, arg: String?): String

    @JvmStatic external fun tuneTo(idx: Int): String

    @JvmStatic external fun toggleFavorite(idx: Int): String

    /** The player failed on tune [seq]: hide the channel and move on. */
    @JvmStatic external fun playbackFailed(seq: Int): String

    /** Tune [seq] is playing: remember the channel works. */
    @JvmStatic external fun playbackStarted(seq: Int): String

    /** "true" if a playlist is already loaded in this process (activity recreated). */
    @JvmStatic external fun isLoaded(): String

    /** The search in effect, or "". */
    @JvmStatic external fun searchText(): String

    /** Shows only channels whose name matches [text] (empty: all channels). */
    @JvmStatic external fun setSearch(text: String): String

    /** Countries / languages with channel counts and the current choice (JSON). */
    @JvmStatic external fun regionsInfo(): String

    /** Shows only channels from these countries or in these languages (JSON arrays); effects. */
    @JvmStatic external fun setRegions(countries: String, languages: String): String

    /** Preferred track languages: `{"audio": code|null, "subtitles": code|null}`. */
    @JvmStatic external fun trackPrefs(): String

    /** Remembers the audio / subtitle language picked (null: stream default / subtitles off). */
    @JvmStatic external fun setTrackPrefs(audio: String?, subtitles: String?): String

    /** "true" if the phone remote was turned on before (it then starts with the app). */
    @JvmStatic external fun remoteEnabled(): String

    /** Starts the phone remote once: `{"address","pin","qrSize","qr":"0101..."}`. */
    @JvmStatic external fun remoteStart(): String

    /** The player's tracks and volume for phones: `{"audio":[{id,label,selected}],"subs":[...],"volume","muted"}`. */
    @JvmStatic external fun remotePlayer(info: String): String

    /** Waits up to [timeoutMs] for a phone's request (JSON with "cmd"), or "". Blocking: own thread. */
    @JvmStatic external fun remoteNext(timeoutMs: Int): String

    /** Channel list header: `{"title","status","current"}`. */
    @JvmStatic external fun listInfo(): String

    /** Channel indices in the current view, ascending. Cheap. */
    @JvmStatic external fun view(): IntArray

    /** Indices of favorite channels. */
    @JvmStatic external fun favorites(): IntArray

    /** Every channel name by index. Call once, off the main thread. */
    @JvmStatic external fun names(): Array<String>

    /** Merges background check results: `{"changed":bool,"status":String?}`. */
    @JvmStatic external fun poll(): String

    /** Newer release: `{}` if up to date, else `{"version","fileName","url","sha256"}`. Blocking. */
    @JvmStatic external fun checkUpdate(currentVersion: String): String

    /** Downloads and verifies an update (JSON from [checkUpdate]) to [destPath]. Blocking. */
    @JvmStatic external fun downloadUpdate(update: String, destPath: String): String

    /** Persists the last channel, favorites and check results. */
    @JvmStatic external fun save(): String
}
