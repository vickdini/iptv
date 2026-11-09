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

    /** Persists the last channel, favorites and check results. */
    @JvmStatic external fun save(): String
}
