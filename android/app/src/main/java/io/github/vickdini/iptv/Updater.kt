package io.github.vickdini.iptv

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import org.json.JSONObject
import java.io.File

/**
 * Automatic updates from GitHub releases.
 *
 * On every launch [check] looks for a newer `IPTV-<version>.apk` and
 * downloads it in the background (the shared Rust code verifies its
 * SHA-256; Android additionally verifies it is signed with the same key).
 * Then [install] hands it to Android's package installer:
 *
 * - Android 12+ can update silently once IPTV is the app that installed
 *   itself; that is done when the user leaves the app, so nothing is
 *   interrupted.
 * - Otherwise (first self-update after sideloading, Fire TV, older Android)
 *   Android shows its "Update this app?" confirmation right away.
 */
object Updater {
    private const val TAG = "IPTV"

    /** A verified APK waiting to be installed. */
    var ready: Ready? = null
        private set

    class Ready(val version: String, val apk: File)

    /** Blocking: checks for and downloads a newer version. Call off the main thread. */
    fun check(context: Context): Ready? {
        val current = context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: return null
        val dir = File(context.cacheDir, "updates")
        dir.listFiles()?.forEach { it.delete() } // leftovers from earlier updates
        val json = runCatching { Core.checkUpdate(current) }
            .onFailure { Log.d(TAG, "update check failed: ${it.message}") }
            .getOrNull() ?: return null
        val update = JSONObject(json)
        if (!update.has("version")) {
            Log.d(TAG, "up to date ($current)")
            return null
        }
        val apk = File(dir, update.getString("fileName"))
        return runCatching {
            Core.downloadUpdate(json, apk.absolutePath)
            Ready(update.getString("version"), apk).also { ready = it }
        }.onFailure { Log.d(TAG, "update download failed: ${it.message}") }.getOrNull()
    }

    /** True if Android can install it without asking (see class docs). */
    fun canInstallSilently(context: Context): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return false
        val installer = runCatching {
            context.packageManager.getInstallSourceInfo(context.packageName).installingPackageName
        }.getOrNull()
        return installer == context.packageName
    }

    /** False if the user must first allow IPTV to install apps (opens that setting). */
    fun ensurePermission(activity: Activity): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return true
        if (activity.packageManager.canRequestPackageInstalls()) return true
        runCatching {
            activity.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}"))
            )
        }
        return false
    }

    /** Hands the downloaded APK to Android's installer. */
    fun install(context: Context) {
        val r = ready ?: return
        ready = null
        val packageInstaller = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val id = packageInstaller.createSession(params)
        packageInstaller.openSession(id).use { session ->
            session.openWrite("base.apk", 0, r.apk.length()).use { out ->
                r.apk.inputStream().use { it.copyTo(out) }
                session.fsync(out)
            }
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
            val callback = PendingIntent.getBroadcast(
                context, id, Intent(context, UpdateReceiver::class.java), flags
            )
            session.commit(callback.intentSender)
        }
        Log.d(TAG, "installing update ${r.version}")
    }
}

/** Receives the installer's result; shows Android's confirmation when needed. */
class UpdateReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, -999)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                runCatching { context.startActivity(confirm) }
            }
            PackageInstaller.STATUS_SUCCESS -> Log.d("IPTV", "update installed")
            else -> Log.d("IPTV", "update failed: $status ${intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)}")
        }
    }
}
