package io.github.vickdini.iptv

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaStyleNotificationHelper

/**
 * Keeps IPTV playing while the screen is off or the phone is locked.
 *
 * The player itself stays in [MainActivity]; this "media playback" foreground
 * service only tells Android the app is in use, so it isn't frozen or killed
 * in the background. Its notification (the media controls on the lock screen
 * and in the notification shade) shows the channel, with previous / next
 * channel and stop.
 */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
class PlaybackService : Service() {

    companion object {
        private const val CHANNEL = "playback"
        private const val ID = 7

        /** The activity's media session (lock screen controls, headphone buttons). */
        var session: MediaSession? = null
        private var title = ""
        private var running = false

        /** Starts (or updates) the service. Call while the app is on screen. */
        fun start(context: Context, channel: String) {
            title = channel
            runCatching {
                ContextCompat.startForegroundService(context, Intent(context, PlaybackService::class.java))
            }
        }

        /** The channel changed: show it in the notification. */
        fun update(context: Context, channel: String) {
            title = channel
            if (!running) return
            runCatching {
                context.getSystemService(NotificationManager::class.java).notify(ID, build(context))
            }
        }

        fun stop(context: Context) {
            runCatching { context.stopService(Intent(context, PlaybackService::class.java)) }
        }

        private fun build(context: Context): Notification {
            fun action(name: String, code: Int) = PendingIntent.getBroadcast(
                context, 10 + code,
                Intent(MainActivity.PIP_ACTION).setPackage(context.packageName).putExtra("action", name),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val open = PendingIntent.getActivity(
                context, 0,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val b = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title.ifEmpty { "IPTV" })
                .setContentText("IPTV")
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                .setCategory(NotificationCompat.CATEGORY_TRANSPORT)
                .addAction(R.drawable.ic_pip_prev, context.getString(R.string.pip_down), action("down", 1))
                .addAction(R.drawable.ic_pip_next, context.getString(R.string.pip_up), action("up", 2))
                .addAction(R.drawable.ic_exit, context.getString(R.string.stop), action("stop", 3))
            session?.let {
                b.setStyle(MediaStyleNotificationHelper.MediaStyle(it).setShowActionsInCompactView(0, 1, 2))
            }
            return b.build()
        }
    }

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26 && nm.getNotificationChannel(CHANNEL) == null) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, getString(R.string.playback), NotificationManager.IMPORTANCE_LOW)
                    .apply { setShowBadge(false) },
            )
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0
        runCatching { ServiceCompat.startForeground(this, ID, build(this), type) }
            .onFailure { stopSelf() }
            .onSuccess { running = true }
        return START_NOT_STICKY
    }

    /** The app was swiped away from the recent apps: stop playing. */
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
