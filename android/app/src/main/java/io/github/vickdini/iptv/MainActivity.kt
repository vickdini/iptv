package io.github.vickdini.iptv

import android.annotation.SuppressLint
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.GestureDetector
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import androidx.core.widget.doAfterTextChanged
import android.widget.FrameLayout
import android.widget.TextView
import androidx.activity.OnBackPressedCallback
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.MediaItem
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.common.Tracks
import java.util.Locale
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import androidx.media3.cast.CastPlayer
import androidx.media3.cast.SessionAvailabilityListener
import androidx.media3.common.MediaMetadata
import androidx.mediarouter.app.MediaRouteButton
import android.content.pm.PackageManager
import android.app.PendingIntent
import android.app.PictureInPictureParams
import android.app.RemoteAction
import android.app.UiModeManager
import android.content.ActivityNotFoundException
import androidx.activity.result.contract.ActivityResultContracts
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.drawable.Icon
import android.util.Rational
import androidx.core.content.ContextCompat
import androidx.media3.common.VideoSize
import android.graphics.Bitmap
import android.graphics.Color
import android.media.AudioManager
import android.widget.ImageView
import android.widget.LinearLayout
import androidx.appcompat.app.AlertDialog
import android.content.res.Configuration
import com.google.android.gms.cast.framework.CastButtonFactory
import com.google.android.gms.cast.framework.CastContext
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors
import kotlin.math.abs

/**
 * Full-screen TV. All channel logic is in the shared Rust [Core]; this class
 * plays streams with ExoPlayer and turns touches / remote keys into actions.
 *
 * Phone: swipe up/down = change channel, tap = channel list, double-tap =
 * previous channel, long-press = info. Remote: Up/Down = channel, OK = list
 * (or confirm a typed number), Left/Right = group, digits = channel number.
 *
 * Phones and tablets held upright show two channels, one above the other
 * ([Pane]s): the "active" one has the sound and is the core's current
 * channel; the other zaps on its own. Leaving the app keeps the active
 * channel playing in a picture-in-picture window.
 */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
class MainActivity : AppCompatActivity() {

    companion object {
        const val PLAYLIST_URL = "https://iptv-org.github.io/iptv/index.m3u"
        private const val TAG = "IPTV"
        private const val PIP_ACTION = "io.github.vickdini.iptv.PIP"
    }

    /** One picture: a player, its view and what it plays. */
    private inner class Pane(val frame: FrameLayout, view: PlayerView, val label: TextView) {
        val exo: ExoPlayer = ExoPlayer.Builder(this@MainActivity).build().also { view.player = it }
        /** The stream it plays (a core "tune" effect or [Core.streamInfo]). */
        var tune: JSONObject? = null
        var idx = -1
        var triedHls = false
        /** Second picture: direction of the last zap, and failures in a row. */
        var forward = true
        var fails = 0
        val hideLabel = Runnable { label.visibility = View.GONE }
    }

    private lateinit var panes: Array<Pane>
    /** The pane with the sound: the core's current channel. */
    private var active = 0
    /** Two pictures on screen (phone or tablet held upright). */
    private var split = false
    private val player: ExoPlayer get() = panes[active].exo
    private val currentTune: JSONObject? get() = panes[active].tune
    private val pipReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            intent.getStringExtra("action")?.let { tvAction(it) }
        }
    }
    private lateinit var osd: TextView
    private lateinit var status: TextView
    private lateinit var panel: View
    private lateinit var panelTitle: TextView
    private lateinit var panelStatus: TextView
    private lateinit var list: RecyclerView
    private lateinit var search: EditText
    /** Set while changing the search box from code (not typing). */
    private var settingSearch = false
    private lateinit var adapter: ChannelAdapter
    private lateinit var tracksButton: View
    /** Moving favorites: the system's file pickers. */
    private val saveFavorites = registerForActivityResult(ActivityResultContracts.CreateDocument("application/json")) { uri ->
        if (uri == null) return@registerForActivityResult
        runCatching {
            contentResolver.openOutputStream(uri)?.use { it.write(Core.exportFavorites().toByteArray()) }
        }.onSuccess {
            val n = JSONObject(Core.exportFavorites()).getJSONArray("favorites").length()
            showOsd(getString(R.string.favorites_saved, n), 5000)
        }.onFailure { showOsd(getString(R.string.favorites_failed, it.message ?: ""), 5000) }
    }
    private val openFavorites = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri == null) return@registerForActivityResult
        runCatching {
            val text = contentResolver.openInputStream(uri)?.use { it.readBytes().toString(Charsets.UTF_8) } ?: ""
            perform(Core.importFavorites(text))
        }.onSuccess { refreshList(keepPosition = true) }
            .onFailure { showOsd(getString(R.string.favorites_failed, it.message ?: ""), 5000) }
    }

    /** The thread carrying out phone remote requests, once started. */
    private var remoteThread: Thread? = null
    /** Last tracks / volume sent to phones (sent again only when it changes). */
    private var lastPlayerInfo = ""
    /** Tune [currentSeq] whose language hint was shown. */
    private var hintedSeq = -1

    private val handler = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var loaded = false
    private var currentSeq = 0
    private var digitsPending = false
    /** Chromecast playback; null where Google Play services are missing (Fire TV). */
    private var castPlayer: CastPlayer? = null
    private val casting get() = castPlayer?.isCastSessionAvailable == true
    private var names: Array<String> = emptyArray()
    private var logos: Array<String> = emptyArray()
    private var refreshPending = false
    private var lastRefresh = 0L
    private var lastTouch = 0L

    private val hideOsd = Runnable { osd.visibility = View.GONE }
    private val digitTimeout = Runnable { tvAction("digit-timeout") }
    private val pollTask = object : Runnable {
        override fun run() {
            poll()
            handler.postDelayed(this, 1000)
        }
    }
    private val backClosesPanel = object : OnBackPressedCallback(false) {
        override fun handleOnBackPressed() = hidePanel()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        hideSystemBars()

        osd = findViewById(R.id.osd)
        status = findViewById(R.id.status)
        panel = findViewById(R.id.panel)
        panelTitle = findViewById(R.id.panelTitle)
        panelStatus = findViewById(R.id.panelStatus)
        list = findViewById(R.id.list)
        search = findViewById(R.id.search)
        search.doAfterTextChanged { text ->
            if (settingSearch || !loaded) return@doAfterTextChanged
            Core.setSearch(text?.toString() ?: "")
            refreshList(keepPosition = false, toFirst = true)
        }
        search.setOnEditorActionListener { _, actionId, _ ->
            if (actionId == EditorInfo.IME_ACTION_SEARCH || actionId == EditorInfo.IME_ACTION_DONE) {
                hideKeyboard()
                list.post { (list.layoutManager as LinearLayoutManager).findViewByPosition(0)?.requestFocus() }
                true
            } else false
        }

        adapter = ChannelAdapter(
            onClick = { idx -> perform(Core.tuneTo(idx)) },
            onLongClick = { idx ->
                perform(Core.toggleFavorite(idx))
                refreshList(keepPosition = true)
            },
        )
        list.layoutManager = LinearLayoutManager(this)
        list.adapter = adapter
        list.itemAnimator = null
        findViewById<View>(R.id.groupPrev).setOnClickListener { panelAction("group-prev") }
        findViewById<View>(R.id.groupNext).setOnClickListener { panelAction("group-next") }
        findViewById<View>(R.id.favOnly).apply {
            setOnClickListener { panelAction("fav-only") }
            setOnLongClickListener { moveFavorites(); true }
        }
        findViewById<View>(R.id.recent).setOnClickListener { panelAction("recent") }
        findViewById<View>(R.id.regions).setOnClickListener { chooseRegions() }
        tracksButton = findViewById<View>(R.id.tracks).apply { setOnClickListener { chooseTracks() } }
        findViewById<View>(R.id.phoneRemote).setOnClickListener { showRemote() }
        findViewById<View>(R.id.closePanel).setOnClickListener { hidePanel() }
        // Phones and tablets play full screen with the back gesture hidden, so
        // leaving isn't obvious: offer an exit button. TV remotes have Back/Home.
        if (!isTv()) {
            findViewById<View>(R.id.exitApp).apply {
                visibility = View.VISIBLE
                setOnClickListener { exitApp() }
            }
        }
        onBackPressedDispatcher.addCallback(this, backClosesPanel)

        panes = arrayOf(
            Pane(findViewById(R.id.pane0), findViewById(R.id.player), findViewById(R.id.label0)),
            Pane(findViewById(R.id.pane1), findViewById(R.id.player2), findViewById(R.id.label1)),
        )
        panes.forEach { pane -> pane.exo.addListener(paneListener(pane)) }
        ContextCompat.registerReceiver(this, pipReceiver, IntentFilter(PIP_ACTION), ContextCompat.RECEIVER_NOT_EXPORTED)

        setupCast()
        setupGestures()
        Core.init(cacheDir.absolutePath, filesDir.absolutePath)
        Core.setLanguage(Locale.getDefault().toLanguageTag())
        LogoLoader.init(this)
        load()
    }

    private fun load() {
        status.text = getString(R.string.loading)
        status.visibility = View.VISIBLE
        status.setOnClickListener(null)
        io.execute {
            // Names are fetched once here, off the main thread, for the channel list.
            // If Android recreated this screen (keyboard, rotation, font size...), the
            // core is still loaded in this process: resume instead of reloading.
            val resuming = Core.isLoaded() == "true"
            val result = runCatching {
                if (!resuming) Core.loadPlaylist(PLAYLIST_URL)
                names = Core.names()
                logos = Core.logos()
            }
            runOnUiThread {
                result.onSuccess {
                    adapter.names = names
                    adapter.logos = logos
                    status.visibility = View.GONE
                    loaded = true
                    setSearchText(Core.searchText())
                    perform(Core.start())
                    updateLayout()
                    handler.removeCallbacks(pollTask)
                    handler.post(pollTask)
                    // Turned on once, the phone remote starts with the app.
                    if (Core.remoteEnabled() == "true") startRemote()
                    if (!resuming) checkForUpdate()
                }.onFailure { e ->
                    status.text = getString(R.string.load_failed, e.message ?: "")
                    status.setOnClickListener { load() }
                }
            }
        }
    }

    // --- effects from the core ------------------------------------------------

    /** Carries out the core's instructions; returns true if a number is being typed. */
    private fun perform(effectsJson: String): Boolean {
        val effects = JSONArray(effectsJson)
        var typing = false
        var tuned = -1
        for (i in 0 until effects.length()) {
            val e = effects.getJSONObject(i)
            when (e.getString("type")) {
                "tune" -> { tune(e); tuned = e.getInt("idx") }
                "osd" -> showOsd(e.getString("text"), e.getLong("ms"), logoOf = tuned)
                "digitTimer" -> {
                    typing = true
                    handler.removeCallbacks(digitTimeout)
                    handler.postDelayed(digitTimeout, e.getLong("ms"))
                }
            }
        }
        if (!typing) handler.removeCallbacks(digitTimeout)
        return typing
    }

    private fun tvAction(name: String, arg: String? = null) {
        Log.d(TAG, "action $name ${arg ?: ""} (loaded=$loaded)")
        if (!loaded) return
        digitsPending = perform(Core.action(name, arg))
    }

    private fun tune(e: JSONObject) {
        Log.d(TAG, "tune #${e.getInt("seq")}: ${e.getInt("number")} ${e.getString("name")}")
        currentSeq = e.getInt("seq")
        val pane = panes[active]
        val idx = e.getInt("idx")
        val state = pane.exo.playbackState
        if (!casting && pane.idx == idx && (state == Player.STATE_READY || state == Player.STATE_BUFFERING)) {
            // Already playing here (the sound moved to this half): keep it going.
            pane.tune = e
            if (state == Player.STATE_READY) Core.playbackStarted(currentSeq)
        } else {
            pane.tune = e
            pane.idx = idx
            pane.triedHls = false
            startStream(pane, e, forceHls = false)
        }
        showLabel(pane)
        updatePip()
        if (panel.visibility == View.VISIBLE) adapter.setCurrent(idx)
    }

    private fun startStream(e: JSONObject, forceHls: Boolean) = startStream(panes[active], e, forceHls)

    private fun startStream(pane: Pane, e: JSONObject, forceHls: Boolean) {
        val url = e.getString("url")
        castPlayer?.takeIf { casting }?.let { cast ->
            val item = MediaItem.Builder().setUri(url)
                .setMimeType(if (forceHls || url.contains(".m3u8", ignoreCase = true)) MimeTypes.APPLICATION_M3U8 else null)
                .setMediaMetadata(MediaMetadata.Builder().setTitle("${e.getInt("number")}  ${e.getString("name")}").build())
                .build()
            cast.setMediaItem(item)
            cast.prepare()
            cast.playWhenReady = true
            return
        }
        val http = DefaultHttpDataSource.Factory()
            .setUserAgent(e.getString("userAgent"))
            .setAllowCrossProtocolRedirects(true)
            .setConnectTimeoutMs(10_000)
            .setReadTimeoutMs(15_000)
        if (!e.isNull("referrer")) http.setDefaultRequestProperties(mapOf("Referer" to e.getString("referrer")))
        val item = MediaItem.Builder().setUri(url).apply {
            if (forceHls || url.contains(".m3u8", ignoreCase = true)) setMimeType(MimeTypes.APPLICATION_M3U8)
        }.build()
        applyTrackPrefs(pane.exo)
        pane.exo.setMediaSource(DefaultMediaSourceFactory(http).createMediaSource(item))
        pane.exo.prepare()
        pane.exo.playWhenReady = true
        pane.exo.volume = if (pane === panes[active]) 1f else 0f
    }

    /** Events of one pane's player. Only the active pane reports to the core. */
    private fun paneListener(pane: Pane) = object : Player.Listener {
        override fun onPlayerError(error: PlaybackException) {
            val tune = pane.tune ?: return
            // URLs without an .m3u8 extension may still be HLS: retry once as HLS.
            if (!pane.triedHls && error.errorCode == PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED) {
                pane.triedHls = true
                startStream(pane, tune, forceHls = true)
                return
            }
            if (pane === panes[active]) perform(Core.playbackFailed(currentSeq))
            else if (split && ++pane.fails < 20) zapOther(pane.forward) // quietly try the next one
        }

        override fun onPlaybackStateChanged(state: Int) {
            if (state != Player.STATE_READY) return
            pane.fails = 0
            if (pane === panes[active]) Core.playbackStarted(currentSeq)
        }

        override fun onTracksChanged(tracks: Tracks) {
            if (pane === panes[active]) onTracks()
        }

        override fun onVideoSizeChanged(videoSize: VideoSize) {
            if (pane === panes[active]) updatePip()
        }
    }

    // --- split screen -------------------------------------------------------------

    private val other: Pane get() = panes[1 - active]

    /** Upright phone or tablet: two pictures; otherwise (and in PiP, or casting) one. */
    private fun updateLayout() {
        val portrait = resources.configuration.orientation == Configuration.ORIENTATION_PORTRAIT
        val want = loaded && portrait && !isTv() && !casting && !inPip()
        if (want != split) {
            split = want
            if (split) {
                if (other.idx < 0) {
                    // Start with the previous channel, else the next one.
                    val prev = Core.previousChannel().toInt()
                    val first = if (prev >= 0 && prev != panes[active].idx) prev
                        else Core.neighbor(panes[active].idx.coerceAtLeast(0), 1).toInt()
                    if (first >= 0) playIn(other, first)
                } else {
                    other.tune?.let { startStream(other, it, forceHls = other.triedHls) }
                }
                showOsd(getString(R.string.split_hint), 5000)
            } else {
                other.exo.stop()
            }
        }
        panes.forEachIndexed { i, pane ->
            pane.frame.visibility = if (split || i == active) View.VISIBLE else View.GONE
            pane.frame.foreground = if (split && i == active) ContextCompat.getDrawable(this, R.drawable.pane_active) else null
            pane.exo.volume = if (i == active) 1f else 0f
            if (!split) pane.label.visibility = View.GONE
        }
    }

    /** Plays a channel in the second picture (the core's channel doesn't change). */
    private fun playIn(pane: Pane, idx: Int) {
        val e = JSONObject(Core.streamInfo(idx))
        if (!e.has("url")) return
        pane.tune = e
        pane.idx = idx
        pane.triedHls = false
        startStream(pane, e, forceHls = false)
        showLabel(pane)
    }

    /** Swipe on the half without sound: zap it. */
    private fun zapOther(forward: Boolean) {
        val pane = other
        pane.forward = forward
        val next = Core.neighbor(pane.idx.coerceAtLeast(0), if (forward) 1 else 0).toInt()
        if (next >= 0) playIn(pane, next)
    }

    /** Tap on the half without sound: it gets the sound, the channel list and the remote. */
    private fun activate(i: Int) {
        if (i == active || !split) return
        active = i
        updateLayout()
        val idx = panes[active].idx
        if (idx >= 0) perform(Core.tuneTo(idx)) // same channel: keeps playing
        lastPlayerInfo = ""
        onTracks()
    }

    /** Channel number and name at the bottom of a half, for a few seconds. */
    private fun showLabel(pane: Pane) {
        val e = pane.tune ?: return
        if (!split) return
        pane.label.text = "${e.getInt("number")}  ${e.getString("name")}"
        pane.label.visibility = View.VISIBLE
        handler.removeCallbacks(pane.hideLabel)
        handler.postDelayed(pane.hideLabel, 3000)
    }

    /** Which half a touch at [y] (in the gesture layer) is on. */
    private fun paneAt(y: Float): Int {
        if (!split) return active
        val h = findViewById<View>(R.id.touch).height
        return if (y < h / 2f) 0 else 1
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        updateLayout()
    }

    // --- picture-in-picture --------------------------------------------------------

    private fun inPip() = android.os.Build.VERSION.SDK_INT >= 24 && isInPictureInPictureMode

    /** Phones and tablets keep playing in a small window when the user leaves the app. */
    private fun canPip() = android.os.Build.VERSION.SDK_INT >= 26 && !isTv() && loaded && !casting &&
        currentTune != null && packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)

    @androidx.annotation.RequiresApi(26)
    private fun pipParams(): PictureInPictureParams {
        val size = player.videoSize
        val ratio = if (size.width > 0 && size.height > 0)
            (size.width * size.pixelWidthHeightRatio / size.height).coerceIn(0.42f, 2.38f) else 16f / 9f
        fun action(name: String, icon: Int, title: Int, code: Int) = RemoteAction(
            Icon.createWithResource(this, icon), getString(title), getString(title),
            PendingIntent.getBroadcast(
                this, code, Intent(PIP_ACTION).setPackage(packageName).putExtra("action", name),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            ),
        )
        val b = PictureInPictureParams.Builder()
            .setAspectRatio(Rational((ratio * 1000).toInt(), 1000))
            .setActions(listOf(
                action("down", R.drawable.ic_pip_prev, R.string.pip_down, 1),
                action("up", R.drawable.ic_pip_next, R.string.pip_up, 2),
            ))
        if (android.os.Build.VERSION.SDK_INT >= 31) b.setAutoEnterEnabled(canPip()).setSeamlessResizeEnabled(true)
        return b.build()
    }

    /** Keeps the PiP window's shape and (Android 12+) automatic entry up to date. */
    private fun updatePip() {
        if (android.os.Build.VERSION.SDK_INT >= 26 && !isTv() &&
            packageManager.hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)
        ) runCatching { setPictureInPictureParams(pipParams()) }
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        // Android 8-11: enter PiP ourselves (12+ does it automatically).
        if (android.os.Build.VERSION.SDK_INT in 26..30 && canPip()) {
            runCatching { enterPictureInPictureMode(pipParams()) }
        }
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        if (isInPictureInPictureMode) {
            hidePanel()
            osd.visibility = View.GONE
        }
        updateLayout()
    }

    /** [logoOf]: the channel whose logo goes with the message (the banner), or -1. */
    private fun showOsd(text: String, ms: Long, logoOf: Int = -1) {
        Log.d(TAG, "osd: " + text.replace("\n", " | "))
        osd.setCompoundDrawablesRelative(null, null, null, null)
        osd.tag = text
        logos.getOrNull(logoOf)?.takeIf { it.isNotEmpty() }?.let { url ->
            LogoLoader.load(url) { bmp ->
                if (bmp == null || osd.tag != text || osd.visibility != View.VISIBLE) return@load
                val h = (40 * resources.displayMetrics.density).toInt()
                val w = (bmp.width * h / bmp.height.toFloat()).toInt().coerceIn(1, h * 3)
                val d = android.graphics.drawable.BitmapDrawable(resources, bmp).apply { setBounds(0, 0, w, h) }
                osd.compoundDrawablePadding = (12 * resources.displayMetrics.density).toInt()
                osd.setCompoundDrawablesRelative(d, null, null, null)
            }
        }
        // Keep messages clear of the channel list.
        (osd.layoutParams as FrameLayout.LayoutParams).gravity =
            Gravity.TOP or if (panel.visibility == View.VISIBLE) Gravity.END else Gravity.START
        osd.requestLayout()
        osd.text = text
        osd.visibility = View.VISIBLE
        handler.removeCallbacks(hideOsd)
        handler.postDelayed(hideOsd, ms)
    }

    // --- channel list -----------------------------------------------------------

    private fun showPanel() {
        if (!loaded) return
        panel.visibility = View.VISIBLE
        backClosesPanel.isEnabled = true
        osd.visibility = View.GONE
        refreshList(keepPosition = false)
    }

    private fun hidePanel() {
        panel.visibility = View.GONE
        backClosesPanel.isEnabled = false
    }

    private fun panelAction(name: String) {
        // Switching group / favorites ends the search (the core does the same).
        if (search.text.isNotEmpty()) setSearchText("")
        tvAction(name)
        refreshList(keepPosition = false)
    }

    private fun setSearchText(text: String) {
        settingSearch = true
        search.setText(text)
        settingSearch = false
    }

    private fun hideKeyboard() {
        getSystemService(InputMethodManager::class.java)?.hideSoftInputFromWindow(search.windowToken, 0)
    }

    /** Opens the channel list with the cursor in the search box. */
    private fun startSearch() {
        if (panel.visibility != View.VISIBLE) showPanel()
        search.requestFocus()
        getSystemService(InputMethodManager::class.java)?.showSoftInput(search, InputMethodManager.SHOW_IMPLICIT)
    }

    /**
     * Reloads the list; keeps the scroll position when channels just
     * appeared/disappeared, or jumps to the first row for new search results.
     */
    private fun refreshList(keepPosition: Boolean, toFirst: Boolean = false) {
        val t0 = System.nanoTime()
        val info = JSONObject(Core.listInfo())
        panelTitle.text = info.getString("title")
        panelStatus.text = if (info.isNull("status")) getString(R.string.panel_hint) else info.getString("status")
        val lm = list.layoutManager as LinearLayoutManager
        var anchorIdx = -1
        var anchorTop = 0
        if (keepPosition) {
            val p = lm.findFirstVisibleItemPosition()
            adapter.items.getOrNull(p)?.let {
                anchorIdx = it
                anchorTop = lm.findViewByPosition(p)?.top ?: 0
            }
        }
        val current = info.getInt("current")
        adapter.submit(Core.view(), Core.favorites(), current)
        lastRefresh = SystemClock.uptimeMillis()
        refreshPending = false
        Log.d(TAG, "list refresh: ${adapter.itemCount} rows in ${(System.nanoTime() - t0) / 1_000_000} ms")
        if (toFirst) {
            lm.scrollToPositionWithOffset(0, 0)
        } else if (anchorIdx >= 0) {
            lm.scrollToPositionWithOffset(adapter.positionOf(anchorIdx), anchorTop)
        } else {
            // After layout, so the list knows its height: current channel a third down.
            list.post {
                val pos = adapter.positionOf(current)
                lm.scrollToPositionWithOffset(pos, list.height / 3)
                list.post { lm.findViewByPosition(pos)?.requestFocus() } // for remote controls
            }
        }
    }

    private fun poll() {
        if (!loaded) return
        reportPlayer()
        val r = JSONObject(Core.poll())
        if (r.getBoolean("changed")) refreshPending = true
        if (panel.visibility != View.VISIBLE) return
        val now = SystemClock.uptimeMillis()
        // Channels found dead while checking: update the list now and then,
        // never while the user is touching or scrolling it.
        if (refreshPending && now - lastRefresh > 2000 && now - lastTouch > 1500 &&
            list.scrollState == RecyclerView.SCROLL_STATE_IDLE
        ) {
            refreshList(keepPosition = true)
        } else if (!r.isNull("status")) {
            panelStatus.text = r.getString("status")
        }
    }

    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        lastTouch = SystemClock.uptimeMillis()
        return super.dispatchTouchEvent(ev)
    }

    // --- input ------------------------------------------------------------------

    @SuppressLint("ClickableViewAccessibility")
    private fun setupGestures() {
        val detector = GestureDetector(this, object : GestureDetector.SimpleOnGestureListener() {
            override fun onDown(e: MotionEvent) = true

            override fun onSingleTapConfirmed(e: MotionEvent): Boolean {
                val pane = paneAt(e.y)
                when {
                    pane != active && panel.visibility != View.VISIBLE -> activate(pane)
                    panel.visibility == View.VISIBLE -> hidePanel()
                    else -> showPanel()
                }
                return true
            }

            override fun onDoubleTap(e: MotionEvent): Boolean {
                tvAction("prev")
                return true
            }

            override fun onLongPress(e: MotionEvent) = tvAction("info")

            override fun onFling(e1: MotionEvent?, e2: MotionEvent, vx: Float, vy: Float): Boolean {
                if (abs(vy) > abs(vx) && abs(vy) > 500) {
                    // Swipe up = next channel, on the half where the swipe started.
                    if (paneAt(e1?.y ?: e2.y) == active) tvAction(if (vy < 0) "up" else "down")
                    else zapOther(forward = vy < 0)
                    return true
                }
                return false
            }
        })
        findViewById<View>(R.id.touch).setOnTouchListener { _, ev -> detector.onTouchEvent(ev) }
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        // While typing a search, keys are text, not remote-control shortcuts.
        if (search.hasFocus() && event.keyCode != KeyEvent.KEYCODE_BACK) return super.dispatchKeyEvent(event)
        if (event.action == KeyEvent.ACTION_DOWN && loaded && handleKey(event.keyCode)) return true
        return super.dispatchKeyEvent(event)
    }

    /** Remote-control / keyboard keys. Returns true if handled. */
    private fun handleKey(code: Int): Boolean {
        val digit = when (code) {
            in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> code - KeyEvent.KEYCODE_0
            in KeyEvent.KEYCODE_NUMPAD_0..KeyEvent.KEYCODE_NUMPAD_9 -> code - KeyEvent.KEYCODE_NUMPAD_0
            else -> -1
        }
        if (digit >= 0) {
            tvAction("digit", digit.toString())
            return true
        }
        val panelOpen = panel.visibility == View.VISIBLE
        when (code) {
            KeyEvent.KEYCODE_CHANNEL_UP, KeyEvent.KEYCODE_PAGE_UP,
            KeyEvent.KEYCODE_MEDIA_NEXT, KeyEvent.KEYCODE_MEDIA_FAST_FORWARD -> tvAction("up")
            KeyEvent.KEYCODE_CHANNEL_DOWN, KeyEvent.KEYCODE_PAGE_DOWN,
            KeyEvent.KEYCODE_MEDIA_PREVIOUS, KeyEvent.KEYCODE_MEDIA_REWIND -> tvAction("down")
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE -> if (panelOpen) hidePanel() else showPanel()
            KeyEvent.KEYCODE_DPAD_LEFT -> if (panelOpen) panelAction("group-prev") else tvAction("group-prev")
            KeyEvent.KEYCODE_DPAD_RIGHT -> if (panelOpen) panelAction("group-next") else tvAction("group-next")
            KeyEvent.KEYCODE_LAST_CHANNEL, KeyEvent.KEYCODE_DEL -> tvAction("prev")
            KeyEvent.KEYCODE_INFO, KeyEvent.KEYCODE_I -> tvAction("info")
            KeyEvent.KEYCODE_BOOKMARK, KeyEvent.KEYCODE_S -> tvAction("fav")
            KeyEvent.KEYCODE_MENU, KeyEvent.KEYCODE_GUIDE, KeyEvent.KEYCODE_L ->
                if (panelOpen) hidePanel() else showPanel()
            KeyEvent.KEYCODE_ESCAPE -> if (panelOpen) hidePanel() else return false
            KeyEvent.KEYCODE_SEARCH, KeyEvent.KEYCODE_SLASH -> startSearch()
            KeyEvent.KEYCODE_MEDIA_AUDIO_TRACK, KeyEvent.KEYCODE_A -> nextTrack(audio = true)
            KeyEvent.KEYCODE_CAPTIONS, KeyEvent.KEYCODE_T -> nextTrack(audio = false)
            // With the list open, Up/Down/OK move through it and pick a channel.
            KeyEvent.KEYCODE_DPAD_UP -> if (panelOpen) return false else tvAction("up")
            KeyEvent.KEYCODE_DPAD_DOWN -> if (panelOpen) return false else tvAction("down")
            KeyEvent.KEYCODE_DPAD_CENTER, KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> when {
                digitsPending -> tvAction("ok")
                panelOpen -> return false
                else -> showPanel()
            }
            else -> return false
        }
        return true
    }

    // --- audio language & subtitles ----------------------------------------------------

    /** One audio or subtitle track of the current stream. */
    private class Track(val group: Tracks.Group, val index: Int) {
        val format: Format get() = group.getTrackFormat(index)
        val selected get() = group.isTrackSelected(index)
        val language: String? get() = format.language?.takeIf { it.isNotBlank() && it != C.LANGUAGE_UNDETERMINED }

        /** The broadcaster's name for it, else the language's name (in the phone's language). */
        fun label(n: Int): String {
            format.label?.takeIf { it.isNotBlank() }?.let { return it }
            val lang = language ?: return "#$n"
            val name = Locale.forLanguageTag(lang).getDisplayLanguage(Locale.getDefault())
            return if (name.isBlank() || name.equals(lang, ignoreCase = true)) lang.uppercase() else
                name.replaceFirstChar { it.titlecase(Locale.getDefault()) }
        }
    }

    private fun tracksOf(type: Int): List<Track> =
        player.currentTracks.groups.filter { it.type == type }.flatMap { g ->
            (0 until g.length).filter { g.isTrackSupported(it) }.map { Track(g, it) }
        }

    /** Before each stream: the remembered languages, no leftover manual choice. */
    private fun applyTrackPrefs(exo: ExoPlayer) {
        val prefs = JSONObject(Core.trackPrefs())
        val audio = prefs.optString("audio").takeIf { !prefs.isNull("audio") && it.isNotEmpty() }
        val subs = prefs.optString("subtitles").takeIf { !prefs.isNull("subtitles") && it.isNotEmpty() }
        exo.trackSelectionParameters = exo.trackSelectionParameters.buildUpon()
            .clearOverrides()
            .setPreferredAudioLanguage(audio)
            .setPreferredTextLanguage(subs)
            .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, subs == null)
            .build()
        tracksButton.visibility = View.GONE
    }

    /** The stream's tracks are known: offer the choice when there is one. */
    private fun onTracks() {
        val audio = tracksOf(C.TRACK_TYPE_AUDIO).size
        val subs = tracksOf(C.TRACK_TYPE_TEXT).size
        val choice = audio > 1 || subs > 0
        tracksButton.visibility = if (choice) View.VISIBLE else View.GONE
        reportPlayer()
        if (!choice || hintedSeq == currentSeq || casting) return
        hintedSeq = currentSeq
        val parts = listOfNotNull(
            if (audio > 1) getString(R.string.tracks_hint_audio, audio) else null,
            if (subs > 0) getString(R.string.tracks_hint_subs, subs) else null,
        ).joinToString(" · ")
        // Wait for the channel banner to go first.
        handler.postDelayed({
            if (hintedSeq == currentSeq && panel.visibility != View.VISIBLE)
                showOsd(getString(if (isTv()) R.string.tracks_hint_tv else R.string.tracks_hint, parts), 5000)
        }, 3000)
    }

    /** Menu: audio language or subtitles (straight to the one there is). */
    private fun chooseTracks() {
        val audio = tracksOf(C.TRACK_TYPE_AUDIO)
        val subs = tracksOf(C.TRACK_TYPE_TEXT)
        when {
            audio.size > 1 && subs.isNotEmpty() -> {
                val current = { list: List<Track>, off: String ->
                    list.indexOfFirst { it.selected }.let { if (it < 0) off else list[it].label(it + 1) }
                }
                val items = arrayOf(
                    "${getString(R.string.tracks_audio)}: ${current(audio, "—")}",
                    "${getString(R.string.tracks_subtitles)}: ${current(subs, getString(R.string.tracks_off))}",
                )
                AlertDialog.Builder(this).setTitle(R.string.tracks)
                    .setItems(items) { _, which -> pickTrack(audio = which == 0) }
                    .show()
            }
            audio.size > 1 -> pickTrack(audio = true)
            subs.isNotEmpty() -> pickTrack(audio = false)
            else -> showOsd(getString(R.string.tracks_none), 2500)
        }
    }

    private fun pickTrack(audio: Boolean) {
        val list = tracksOf(if (audio) C.TRACK_TYPE_AUDIO else C.TRACK_TYPE_TEXT)
        // Subtitles start with "Off".
        val labels = (if (audio) emptyList() else listOf(getString(R.string.tracks_off))) +
            list.mapIndexed { i, t -> t.label(i + 1) }
        val sel = list.indexOfFirst { it.selected }
        val checked = if (audio) sel else sel + 1
        AlertDialog.Builder(this)
            .setTitle(if (audio) R.string.tracks_audio else R.string.tracks_subtitles)
            .setSingleChoiceItems(labels.toTypedArray(), checked) { d, which ->
                d.dismiss()
                selectTrack(audio, if (audio) list[which] else list.getOrNull(which - 1))
            }
            .show()
    }

    /** Audio / CC remote keys: the next audio track, or the next subtitles then off. */
    private fun nextTrack(audio: Boolean) {
        val list = tracksOf(if (audio) C.TRACK_TYPE_AUDIO else C.TRACK_TYPE_TEXT)
        if ((audio && list.size < 2) || (!audio && list.isEmpty())) {
            showOsd(getString(R.string.tracks_none), 2500)
            return
        }
        val pos = list.indexOfFirst { it.selected }
        val next = if (audio) list[(pos + 1) % list.size] else list.getOrNull(pos + 1)
        selectTrack(audio, next)
    }

    /** Plays [track] (null: subtitles off) and remembers its language for other channels. */
    private fun selectTrack(audio: Boolean, track: Track?) {
        val type = if (audio) C.TRACK_TYPE_AUDIO else C.TRACK_TYPE_TEXT
        val b = player.trackSelectionParameters.buildUpon().clearOverridesOfType(type)
        if (track == null) {
            b.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
        } else {
            b.setTrackTypeDisabled(type, false)
                .setOverrideForType(TrackSelectionOverride(track.group.mediaTrackGroup, track.index))
            if (audio) b.setPreferredAudioLanguage(track.language) else b.setPreferredTextLanguage(track.language)
        }
        player.trackSelectionParameters = b.build()

        val prefs = JSONObject(Core.trackPrefs())
        fun pref(key: String) = if (prefs.isNull(key)) null else prefs.optString(key)
        if (audio) Core.setTrackPrefs(track?.language ?: pref("audio"), pref("subtitles"))
        else Core.setTrackPrefs(pref("audio"), track?.language)

        val list = tracksOf(type)
        val what = getString(if (audio) R.string.tracks_audio else R.string.tracks_subtitles)
        val text = if (track == null) "$what: ${getString(R.string.tracks_off)}" else {
            val i = list.indexOfFirst { it.group == track.group && it.index == track.index }
            "$what: ${track.label(i + 1)}   (${i + 1}/${list.size})"
        }
        showOsd(text, 2500)
    }

    // --- phone remote ---------------------------------------------------------------

    /** Starts the remote (once) and returns how to connect, or null if it failed. */
    private fun startRemote(): JSONObject? {
        val info = runCatching { JSONObject(Core.remoteStart()) }.getOrElse { e ->
            Log.w(TAG, "phone remote", e)
            showOsd(getString(R.string.remote_failed, e.message ?: ""), 5000)
            return null
        }
        if (remoteThread == null) {
            remoteThread = Thread({
                while (true) {
                    val cmd = runCatching { Core.remoteNext(1000) }.getOrDefault("")
                    if (cmd.isNotEmpty()) runOnUiThread { onRemote(JSONObject(cmd)) }
                }
            }, "phone-remote").apply { isDaemon = true; start() }
        }
        lastPlayerInfo = ""
        reportPlayer()
        return info
    }

    /** The phone remote button: QR code, address and PIN. */
    private fun showRemote() {
        if (!loaded) return
        val info = startRemote() ?: return
        val pad = (16 * resources.displayMetrics.density).toInt()
        val box = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad, pad, 0)
        }
        val message = TextView(this).apply { textSize = 16f }
        if (info.isNull("address") || info.isNull("qr")) {
            message.setText(R.string.remote_no_network)
        } else {
            val size = info.getInt("qrSize")
            val bits = info.getString("qr")
            val quiet = 4
            val side = size + 2 * quiet
            val bmp = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
            bmp.eraseColor(Color.WHITE)
            for (y in 0 until size) for (x in 0 until size) {
                if (bits[y * size + x] == '1') bmp.setPixel(x + quiet, y + quiet, Color.BLACK)
            }
            val px = (240 * resources.displayMetrics.density).toInt()
            box.addView(ImageView(this).apply {
                setImageBitmap(Bitmap.createScaledBitmap(bmp, px, px, false))
                contentDescription = getString(R.string.remote)
            }, LinearLayout.LayoutParams(px, px).apply { gravity = Gravity.CENTER_HORIZONTAL; bottomMargin = pad })
            message.text = getString(R.string.remote_scan, info.getString("address"), info.getString("pin"))
        }
        box.addView(message)
        AlertDialog.Builder(this).setTitle(R.string.remote).setView(box)
            .setPositiveButton(android.R.string.ok, null).show()
    }

    /** Long-press on ★: save the favorites to a file, or add them from one. */
    private fun moveFavorites() {
        if (!loaded) return
        AlertDialog.Builder(this).setTitle(R.string.favorites_move)
            .setItems(arrayOf(getString(R.string.favorites_export), getString(R.string.favorites_import))) { _, which ->
                try {
                    if (which == 0) {
                        if (JSONObject(Core.exportFavorites()).getJSONArray("favorites").length() == 0) {
                            showOsd(getString(R.string.favorites_none), 3000)
                        } else {
                            saveFavorites.launch("IPTV favorites.json")
                        }
                    } else {
                        openFavorites.launch(arrayOf("application/json", "text/plain", "application/octet-stream"))
                    }
                } catch (e: ActivityNotFoundException) {
                    showOsd(getString(R.string.favorites_no_picker), 5000)
                }
            }
            .show()
    }

    /** A request from a phone. */
    private fun onRemote(cmd: JSONObject) {
        if (!loaded) return
        Log.d(TAG, "remote: $cmd")
        when (cmd.optString("cmd")) {
            "action" -> tvAction(cmd.getString("name"), if (cmd.isNull("arg")) null else cmd.optString("arg"))
            "tune" -> {
                perform(Core.tuneTo(cmd.getInt("idx")))
                if (panel.visibility == View.VISIBLE) refreshList(keepPosition = true)
            }
            "fav" -> {
                perform(Core.toggleFavorite(cmd.getInt("idx")))
                if (panel.visibility == View.VISIBLE) refreshList(keepPosition = true)
            }
            "volume" -> audio().adjustStreamVolume(
                AudioManager.STREAM_MUSIC,
                if (cmd.getInt("delta") > 0) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER,
                AudioManager.FLAG_SHOW_UI,
            )
            "addfavorites" -> {
                perform(Core.addFavorites(cmd.getJSONArray("urls").toString(), cmd.optInt("missing")))
                if (panel.visibility == View.VISIBLE) refreshList(keepPosition = true)
            }
            "mute" -> audio().adjustStreamVolume(
                AudioManager.STREAM_MUSIC, AudioManager.ADJUST_TOGGLE_MUTE, AudioManager.FLAG_SHOW_UI,
            )
            "track" -> {
                val isAudio = cmd.optString("kind") == "audio"
                val list = tracksOf(if (isAudio) C.TRACK_TYPE_AUDIO else C.TRACK_TYPE_TEXT)
                val track = if (cmd.isNull("id")) null else list.getOrNull(cmd.getInt("id"))
                if (track != null || !isAudio) selectTrack(isAudio, track)
            }
        }
        reportPlayer()
    }

    private fun audio() = getSystemService(AUDIO_SERVICE) as AudioManager

    /** Sends the tracks (id = position in the list) and volume to phones, if changed. */
    private fun reportPlayer() {
        if (remoteThread == null) return
        fun tracks(type: Int) = JSONArray().apply {
            tracksOf(type).forEachIndexed { i, t ->
                put(JSONObject().put("id", i).put("label", t.label(i + 1)).put("selected", t.selected))
            }
        }
        val am = audio()
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        val info = JSONObject()
            .put("audio", if (casting) JSONArray() else tracks(C.TRACK_TYPE_AUDIO))
            .put("subs", if (casting) JSONArray() else tracks(C.TRACK_TYPE_TEXT))
            .put("volume", am.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 / max)
            .put("muted", am.isStreamMute(AudioManager.STREAM_MUSIC))
            .toString()
        if (info != lastPlayerInfo) {
            lastPlayerInfo = info
            Core.remotePlayer(info)
        }
    }

    // --- countries & languages ------------------------------------------------------

    /** Menu: countries, languages, or show everything again. */
    private fun chooseRegions() {
        if (!loaded) return
        val info = JSONObject(Core.regionsInfo())
        if (info.getJSONArray("countries").length() == 0 && info.getJSONArray("languages").length() == 0) {
            AlertDialog.Builder(this).setMessage(R.string.regions_none).setPositiveButton(android.R.string.ok, null).show()
            return
        }
        val chosenC = info.getJSONArray("chosenCountries")
        val chosenL = info.getJSONArray("chosenLanguages")
        fun summary(label: String, a: JSONArray) = if (a.length() == 0) label else "$label (${a.length()})"
        val items = arrayOf(
            summary(getString(R.string.regions_countries), chosenC),
            summary(getString(R.string.regions_languages), chosenL),
            getString(R.string.regions_show_all),
        )
        AlertDialog.Builder(this)
            .setTitle(R.string.regions)
            .setItems(items) { _, which ->
                when (which) {
                    0 -> pickRegions(info, countries = true)
                    1 -> pickRegions(info, countries = false)
                    else -> applyRegions(emptyList(), emptyList())
                }
            }
            .show()
    }

    /** Checklist of countries (or languages) with their number of working channels. */
    private fun pickRegions(info: JSONObject, countries: Boolean) {
        val all = info.getJSONArray(if (countries) "countries" else "languages")
        val names = Array(all.length()) { all.getJSONArray(it).getString(0) }
        val labels = Array(all.length()) { "${names[it]}  (${all.getJSONArray(it).getInt(1)})" }
        val chosen = jsonList(info.getJSONArray(if (countries) "chosenCountries" else "chosenLanguages"))
        val checked = BooleanArray(names.size) { names[it] in chosen }
        AlertDialog.Builder(this)
            .setTitle(if (countries) R.string.regions_countries else R.string.regions_languages)
            .setMultiChoiceItems(labels, checked) { _, i, on -> checked[i] = on }
            .setPositiveButton(android.R.string.ok) { _, _ ->
                val picked = names.filterIndexed { i, _ -> checked[i] }
                val otherKey = if (countries) "chosenLanguages" else "chosenCountries"
                val other = jsonList(info.getJSONArray(otherKey))
                if (countries) applyRegions(picked, other) else applyRegions(other, picked)
            }
            .setNegativeButton(android.R.string.cancel, null)
            // Clears countries and languages at once: every channel again.
            .setNeutralButton(R.string.regions_clear_all) { _, _ -> applyRegions(emptyList(), emptyList()) }
            .show()
    }

    private fun jsonList(a: JSONArray): List<String> = List(a.length()) { a.getString(it) }

    private fun applyRegions(countries: List<String>, languages: List<String>) {
        perform(Core.setRegions(JSONArray(countries).toString(), JSONArray(languages).toString()))
        refreshList(keepPosition = false)
    }

    /** Android TV, Google TV and Fire TV all report the television UI mode. */
    private fun isTv(): Boolean =
        getSystemService(UiModeManager::class.java)?.currentModeType == Configuration.UI_MODE_TYPE_TELEVISION ||
            packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)

    /** Stops playback (also on a Chromecast), saves, and closes the app. */
    private fun exitApp() {
        player.stop()
        if (casting) {
            runCatching {
                @Suppress("DEPRECATION")
                CastContext.getSharedInstance(this).sessionManager.endCurrentSession(true)
            }
        }
        if (loaded) Core.save()
        finishAndRemoveTask()
    }

    // --- Chromecast -----------------------------------------------------------------

    /**
     * Offers casting on phones and tablets with Google Play services. TV
     * devices are cast targets themselves, and Fire TV has no Play services,
     * so the Cast button stays hidden there.
     */
    private fun setupCast() {
        val button = findViewById<MediaRouteButton>(R.id.castButton)
        if (packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)) return
        val context = try {
            @Suppress("DEPRECATION")
            CastContext.getSharedInstance(this)
        } catch (e: Exception) {
            Log.d(TAG, "casting unavailable: ${e.message}")
            return
        }
        CastButtonFactory.setUpMediaRouteButton(applicationContext, button)
        button.visibility = View.VISIBLE
        val cast = CastPlayer(context)
        castPlayer = cast
        cast.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                // Not castable (e.g. the server blocks cross-origin playback): skip it.
                if (casting && currentTune != null) perform(Core.playbackFailed(currentSeq))
            }

            override fun onPlaybackStateChanged(state: Int) {
                if (casting && state == Player.STATE_READY) Core.playbackStarted(currentSeq)
            }
        })
        cast.setSessionAvailabilityListener(object : SessionAvailabilityListener {
            override fun onCastSessionAvailable() {
                panes.forEach { it.exo.stop() }
                updateLayout()
                currentTune?.let { startStream(it, forceHls = panes[active].triedHls) }
                showOsd(getString(R.string.casting), 3000)
            }

            override fun onCastSessionUnavailable() {
                currentTune?.let { startStream(it, forceHls = panes[active].triedHls) }
                updateLayout()
            }
        })
    }

    // --- updates --------------------------------------------------------------------

    private var updateNeedsPermission = false

    /** Checks GitHub for a newer version and downloads it in the background. */
    private fun checkForUpdate() {
        io.execute {
            val ready = Updater.check(this) ?: return@execute
            runOnUiThread { offerUpdate(ready) }
        }
    }

    private fun offerUpdate(ready: Updater.Ready) {
        if (Updater.canInstallSilently(this)) {
            Log.d(TAG, "update ${ready.version} ready; installing when the app is left")
            return
        }
        showOsd(getString(R.string.update_ready, ready.version), 6000)
        if (!Updater.ensurePermission(this)) {
            updateNeedsPermission = true // install after returning from the setting
            return
        }
        handler.postDelayed({ Updater.install(this) }, 3000)
    }

    override fun onResume() {
        super.onResume()
        if (updateNeedsPermission && Updater.ready != null &&
            (android.os.Build.VERSION.SDK_INT < 26 || packageManager.canRequestPackageInstalls())
        ) {
            updateNeedsPermission = false
            Updater.install(this)
        }
    }

    // --- lifecycle ----------------------------------------------------------------

    override fun onStart() {
        super.onStart()
        if (loaded) {
            if (!casting) {
                currentTune?.let { startStream(it, forceHls = panes[active].triedHls) }
                if (split) other.tune?.let { startStream(other, it, forceHls = other.triedHls) }
            }
            handler.removeCallbacks(pollTask)
            handler.post(pollTask)
        }
    }

    override fun onStop() {
        super.onStop()
        // Still showing in a picture-in-picture window: keep playing. (When that
        // window is closed, onStop runs again and stops it.)
        if (inPip()) return
        // Silent updates happen when the user leaves, so nothing is interrupted.
        if (Updater.ready != null && Updater.canInstallSilently(this)) Updater.install(this)
        panes.forEach { it.exo.stop() } // a Chromecast keeps playing while the phone is locked
        handler.removeCallbacks(pollTask)
        if (loaded) Core.save()
    }

    override fun onDestroy() {
        super.onDestroy()
        panes.forEach { it.exo.release() }
        runCatching { unregisterReceiver(pipReceiver) }
        castPlayer?.release()
        io.shutdown()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) hideSystemBars()
    }

    private fun hideSystemBars() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).run {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
    }
}
