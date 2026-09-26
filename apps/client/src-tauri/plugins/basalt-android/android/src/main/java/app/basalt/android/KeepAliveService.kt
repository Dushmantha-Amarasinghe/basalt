package app.basalt.android

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

/**
 * Keeps the app running while it is doing something the user asked for.
 *
 * Android stops an app it cannot see, and with it the Rust side that is
 * moving the bytes: a 4 GB upload would stop the moment the screen went off,
 * a film handed to VLC would lose the stream it is reading, and music would
 * stop between tracks. A foreground service, with a notification saying what
 * it is doing, is how an app is allowed to carry on.
 *
 * One service for both reasons. It is a media-playback service while
 * anything is playing — those have no time limit — and a data-sync service
 * while only transfers are running. Android 15 allows data-sync six hours a
 * day; past that it asks the service to stop, and it does, and a transfer
 * that was cut short resumes where it stopped next time.
 */
class KeepAliveService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (reasons.isEmpty()) {
      stopSelf()
      return START_NOT_STICKY
    }
    val playback = reasons.containsKey("playback")
    val shown = reasons["playback"] ?: reasons["transfer"] ?: return START_NOT_STICKY
    val type = if (Build.VERSION.SDK_INT >= 29) {
      if (playback) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
      else ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
    } else 0
    try {
      ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(this, shown), type)
    } catch (e: Exception) {
      // Refused — past the day's data-sync allowance, most likely. The work
      // carries on for as long as Android lets it; nothing to crash over.
      stopSelf()
    }
    return START_NOT_STICKY
  }

  override fun onTimeout(startId: Int, fgsType: Int) {
    // Android 15's limit for data-sync: stop, as asked, or be stopped harder.
    reasons.remove("transfer")
    if (reasons.isEmpty()) stopSelf()
  }

  data class Shown(val title: String, val text: String, val progress: Int)

  companion object {
    private const val CHANNEL = "basalt-activity"
    private const val NOTIFICATION_ID = 7742
    private val reasons = linkedMapOf<String, Shown>()

    fun hold(context: Context, reason: String, title: String, text: String, progress: Int) {
      val fresh = !reasons.containsKey(reason)
      reasons[reason] = Shown(title, text, progress)
      if (fresh || reasons.size == 1) {
        val intent = Intent(context, KeepAliveService::class.java)
        runCatching {
          if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent)
          else context.startService(intent)
        }
      } else {
        // Only the notification changes: progress, mostly.
        val shown = reasons["playback"] ?: reasons["transfer"] ?: return
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        runCatching { manager.notify(NOTIFICATION_ID, notification(context, shown)) }
      }
    }

    fun release(context: Context, reason: String) {
      if (reasons.remove(reason) == null) return
      val intent = Intent(context, KeepAliveService::class.java)
      if (reasons.isEmpty()) {
        context.stopService(intent)
      } else {
        runCatching {
          if (Build.VERSION.SDK_INT >= 26) context.startForegroundService(intent)
          else context.startService(intent)
        }
      }
    }

    private fun notification(context: Context, shown: Shown): Notification {
      val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
        manager.createNotificationChannel(
          NotificationChannel(CHANNEL, "Transfers and playback", NotificationManager.IMPORTANCE_LOW).apply {
            description = "Shown while Basalt is copying files or playing."
            setShowBadge(false)
          }
        )
      }
      val open = context.packageManager.getLaunchIntentForPackage(context.packageName)
      val tap = open?.let {
        PendingIntent.getActivity(
          context, 0, it,
          PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
      }
      val icon = context.applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.stat_sys_download
      return NotificationCompat.Builder(context, CHANNEL)
        .setSmallIcon(icon)
        .setContentTitle(shown.title)
        .setContentText(shown.text)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setSilent(true)
        .setContentIntent(tap)
        .apply {
          if (shown.progress in 0..100) setProgress(100, shown.progress, false)
          else if (shown.progress < 0 && !reasons.containsKey("playback")) setProgress(0, 0, true)
        }
        .build()
    }
  }
}
