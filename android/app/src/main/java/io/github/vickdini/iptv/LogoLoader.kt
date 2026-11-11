package io.github.vickdini.iptv

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Handler
import android.os.Looper
import android.util.LruCache
import android.widget.ImageView
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.Executors

/**
 * Channel logos: downloaded a few at a time, shrunk, kept on disk (so each is
 * fetched once) and in memory while the app runs. No library needed.
 */
object LogoLoader {
    private const val MAX_BYTES = 2 * 1024 * 1024
    private const val KEEP_W = 192
    private const val KEEP_H = 96
    /** A logo that failed to load is retried after this long. */
    private const val RETRY_MS = 3L * 24 * 3600 * 1000

    private val memory = object : LruCache<String, Bitmap>(8 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap) = value.byteCount
    }
    private val failed = HashSet<String>()
    private val waiting = HashMap<String, MutableList<(Bitmap?) -> Unit>>()
    private val pool = Executors.newFixedThreadPool(4)
    private val main = Handler(Looper.getMainLooper())
    private var dir: File? = null

    fun init(context: Context) {
        dir = File(context.cacheDir, "logos").apply { mkdirs() }
    }

    /** Calls [done] on the main thread with the logo, or null if there is none. */
    fun load(url: String, done: (Bitmap?) -> Unit) {
        memory.get(url)?.let { done(it); return }
        if (url in failed || !(url.startsWith("http://") || url.startsWith("https://"))) {
            done(null)
            return
        }
        waiting[url]?.let { it.add(done); return } // already loading
        waiting[url] = mutableListOf(done)
        pool.execute {
            val bmp = runCatching { fetch(url) }.getOrNull()
            main.post {
                if (bmp != null) memory.put(url, bmp) else failed.add(url)
                waiting.remove(url)?.forEach { it(bmp) }
            }
        }
    }

    /** Shows the logo at [url] in [view] (recycled list rows are handled). */
    fun into(view: ImageView, url: String?) {
        view.tag = url
        view.setImageDrawable(null)
        if (url.isNullOrEmpty()) return
        load(url) { bmp -> if (view.tag == url) view.setImageBitmap(bmp) }
    }

    private fun fetch(url: String): Bitmap? {
        val base = dir ?: return null
        val name = MessageDigest.getInstance("SHA-1").digest(url.toByteArray())
            .joinToString("") { "%02x".format(it) }
        val cached = File(base, "$name.png")
        val failMark = File(base, "$name.failed")
        if (cached.isFile) BitmapFactory.decodeFile(cached.path)?.let { return it }
        if (failMark.isFile && System.currentTimeMillis() - failMark.lastModified() < RETRY_MS) return null

        val bytes = runCatching {
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = 8_000
            conn.readTimeout = 10_000
            conn.instanceFollowRedirects = true
            conn.setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android) IPTV")
            conn.inputStream.use { input ->
                val buf = java.io.ByteArrayOutputStream()
                val chunk = ByteArray(16 * 1024)
                while (true) {
                    val n = input.read(chunk)
                    if (n < 0) break
                    buf.write(chunk, 0, n)
                    if (buf.size() > MAX_BYTES) error("logo too large")
                }
                buf.toByteArray()
            }
        }.getOrNull()
        val bmp = bytes?.let { decodeSmall(it) }
        if (bmp == null) {
            runCatching { failMark.writeText("") }
            return null
        }
        runCatching { cached.outputStream().use { bmp.compress(Bitmap.CompressFormat.PNG, 100, it) } }
        return bmp
    }

    /** Decodes, shrinking large images to about [KEEP_W] x [KEEP_H]. */
    private fun decodeSmall(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= KEEP_W && bounds.outHeight / (sample * 2) >= KEEP_H) sample *= 2
        val bmp = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
            ?: return null
        val scale = minOf(KEEP_W.toFloat() / bmp.width, KEEP_H.toFloat() / bmp.height, 1f)
        if (scale >= 1f) return bmp
        return Bitmap.createScaledBitmap(bmp, (bmp.width * scale).toInt().coerceAtLeast(1),
            (bmp.height * scale).toInt().coerceAtLeast(1), true)
    }
}
