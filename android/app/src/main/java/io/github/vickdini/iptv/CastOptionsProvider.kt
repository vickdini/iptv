package io.github.vickdini.iptv

import android.content.Context
import com.google.android.gms.cast.CastMediaControlIntent
import com.google.android.gms.cast.framework.CastOptions
import com.google.android.gms.cast.framework.OptionsProvider
import com.google.android.gms.cast.framework.SessionProvider

/**
 * Casts to Google's Default Media Receiver: built into every Chromecast, no
 * registration needed. It plays HLS/MP4 URLs directly; streams that need a
 * custom user agent or whose servers don't allow cross-origin playback
 * (CORS) can't be cast and are skipped like any failing channel.
 */
class CastOptionsProvider : OptionsProvider {
    override fun getCastOptions(context: Context): CastOptions =
        CastOptions.Builder()
            .setReceiverApplicationId(CastMediaControlIntent.DEFAULT_MEDIA_RECEIVER_APPLICATION_ID)
            .build()

    override fun getAdditionalSessionProviders(context: Context): List<SessionProvider>? = null
}
