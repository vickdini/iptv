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
import androidx.media3.common.MediaItem
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
 */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
class MainActivity : AppCompatActivity() {

    companion object {
        const val PLAYLIST_URL = "https://iptv-org.github.io/iptv/index.m3u"
        private const val TAG = "IPTV"
    }

    private lateinit var player: ExoPlayer
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

    private val handler = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var loaded = false
    private var currentTune: JSONObject? = null
    private var currentSeq = 0
    private var triedHls = false
    private var digitsPending = false
    /** Chromecast playback; null where Google Play services are missing (Fire TV). */
    private var castPlayer: CastPlayer? = null
    private val casting get() = castPlayer?.isCastSessionAvailable == true
    private var names: Array<String> = emptyArray()
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
        findViewById<View>(R.id.favOnly).setOnClickListener { panelAction("fav-only") }
        findViewById<View>(R.id.closePanel).setOnClickListener { hidePanel() }
        onBackPressedDispatcher.addCallback(this, backClosesPanel)

        player = ExoPlayer.Builder(this).build()
        findViewById<PlayerView>(R.id.player).player = player
        player.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                val tune = currentTune ?: return
                // URLs without an .m3u8 extension may still be HLS: retry once as HLS.
                if (!triedHls && error.errorCode == PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED) {
                    triedHls = true
                    startStream(tune, forceHls = true)
                    return
                }
                perform(Core.playbackFailed(currentSeq))
            }

            override fun onPlaybackStateChanged(state: Int) {
                if (state == Player.STATE_READY) Core.playbackStarted(currentSeq)
            }
        })

        setupCast()
        setupGestures()
        Core.init(cacheDir.absolutePath, filesDir.absolutePath)
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
            }
            runOnUiThread {
                result.onSuccess {
                    adapter.names = names
                    status.visibility = View.GONE
                    loaded = true
                    setSearchText(Core.searchText())
                    perform(Core.start())
                    handler.removeCallbacks(pollTask)
                    handler.post(pollTask)
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
        for (i in 0 until effects.length()) {
            val e = effects.getJSONObject(i)
            when (e.getString("type")) {
                "tune" -> tune(e)
                "osd" -> showOsd(e.getString("text"), e.getLong("ms"))
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
        currentTune = e
        currentSeq = e.getInt("seq")
        triedHls = false
        startStream(e, forceHls = false)
        if (panel.visibility == View.VISIBLE) adapter.setCurrent(e.getInt("idx"))
    }

    private fun startStream(e: JSONObject, forceHls: Boolean) {
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
        player.setMediaSource(DefaultMediaSourceFactory(http).createMediaSource(item))
        player.prepare()
        player.playWhenReady = true
    }

    private fun showOsd(text: String, ms: Long) {
        Log.d(TAG, "osd: " + text.replace("\n", " | "))
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
                if (panel.visibility == View.VISIBLE) hidePanel() else showPanel()
                return true
            }

            override fun onDoubleTap(e: MotionEvent): Boolean {
                tvAction("prev")
                return true
            }

            override fun onLongPress(e: MotionEvent) = tvAction("info")

            override fun onFling(e1: MotionEvent?, e2: MotionEvent, vx: Float, vy: Float): Boolean {
                if (abs(vy) > abs(vx) && abs(vy) > 500) {
                    tvAction(if (vy < 0) "up" else "down") // swipe up = next channel
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
                player.stop()
                currentTune?.let { startStream(it, forceHls = triedHls) }
                showOsd(getString(R.string.casting), 3000)
            }

            override fun onCastSessionUnavailable() {
                currentTune?.let { startStream(it, forceHls = triedHls) }
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
            if (!casting) currentTune?.let { startStream(it, forceHls = triedHls) }
            handler.removeCallbacks(pollTask)
            handler.post(pollTask)
        }
    }

    override fun onStop() {
        super.onStop()
        // Silent updates happen when the user leaves, so nothing is interrupted.
        if (Updater.ready != null && Updater.canInstallSilently(this)) Updater.install(this)
        player.stop() // a Chromecast keeps playing while the phone is locked
        handler.removeCallbacks(pollTask)
        if (loaded) Core.save()
    }

    override fun onDestroy() {
        super.onDestroy()
        player.release()
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
