package app.basalt.android

import android.app.Activity
import android.graphics.Color
import android.os.SystemClock
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import android.webkit.WebView
import android.widget.FrameLayout
import app.tauri.plugin.JSObject
import dev.jdtech.mpv.MPVLib
import org.json.JSONObject
import java.io.File

/**
 * The player: mpv, drawing on a surface behind the page.
 *
 * The same arrangement as the desktop, where mpv draws into the window behind
 * a see-through page and the controls are ordinary HTML on top. Here the
 * surface sits beneath the WebView in the activity, and the WebView is clear
 * wherever the page is — which is only while something is playing.
 *
 * It answers the same few requests the desktop's mpv plugin does, and sends
 * the same property changes, so the player's code in the page is the one the
 * desktop runs.
 */
class MpvPlayer(
  private val activity: Activity,
  private val emit: (JSObject) -> Unit,
) : MPVLib.EventObserver, SurfaceHolder.Callback {
  private var started = false
  private var surface: SurfaceView? = null
  private var lastTime = 0L

  /**
   * Starts mpv, once for the life of the app, with the page's options — less
   * the ones that only make sense on a desktop — and Android's own.
   */
  fun init(options: Map<String, String>, properties: Map<String, String>, webView: WebView?) {
    if (started) {
      properties.forEach { (name, format) -> MPVLib.observeProperty(name, formatOf(format)) }
      return
    }
    MPVLib.create(activity.applicationContext)

    val desktopOnly = setOf("vo", "hwdec", "audio-device", "gpu-context", "gpu-api")
    for ((name, value) in options) {
      if (name !in desktopOnly) MPVLib.setOptionString(name, value)
    }
    // A phone's GPU, its hardware decoders, its audio.
    //
    // No picture output until there is a surface to draw on: `surfaceCreated`
    // switches it on. Starting with `gpu` meant a video opened before Android
    // had handed the surface over failed to set up its output, and mpv then
    // dropped the video track for good: the first video after pairing played
    // as sound alone. With `null` the track stays, and the picture appears
    // the moment the surface does.
    MPVLib.setOptionString("vo", "null")
    MPVLib.setOptionString("gpu-context", "android")
    MPVLib.setOptionString("opengl-es", "yes")
    MPVLib.setOptionString("hwdec", "mediacodec,mediacodec-copy")
    MPVLib.setOptionString("hwdec-codecs", "h264,hevc,mpeg4,mpeg2video,vp8,vp9,av1")
    MPVLib.setOptionString("ao", "audiotrack,opensles")
    MPVLib.setOptionString("audio-set-media-role", "yes")
    // Streaming from the host through the app's own proxy: a generous read
    // ahead, so Wi-Fi hiccups are ridden out rather than paused for.
    MPVLib.setOptionString("cache", "yes")
    MPVLib.setOptionString("demuxer-max-bytes", "96MiB")
    MPVLib.setOptionString("demuxer-max-back-bytes", "32MiB")
    // Never a window of its own for sound alone: an empty one reports a
    // picture size, and music was taken for a film and turned sideways. The
    // page draws the black stage music plays on.
    MPVLib.setOptionString("force-window", "no")
    MPVLib.setOptionString("idle", "yes")
    // Subtitles need a font to draw with. This build of mpv cannot look fonts
    // up on the phone, so libass falls back to `subfont.ttf` in mpv's config
    // directory, and with none there a chosen subtitle drew nothing at all.
    // `config` must be on for the directory to be searched; there is no
    // mpv.conf in it, so nothing else is read.
    val configDir = File(activity.filesDir, "mpv")
    installSubtitleFont(configDir)
    MPVLib.setOptionString("config", "yes")
    MPVLib.setOptionString("config-dir", configDir.path)
    MPVLib.init()

    MPVLib.addObserver(this)
    properties.forEach { (name, format) -> MPVLib.observeProperty(name, formatOf(format)) }
    started = true

    attachSurface(webView)
  }

  /**
   * Copies one of the phone's own fonts in as `subfont.ttf`, once.
   *
   * Taken from the system rather than shipped in the app: every Android phone
   * has a sans serif to read subtitles in, and this costs the download
   * nothing. Roboto first, which nearly every phone has, then the fonts
   * phones use in its place.
   */
  private fun installSubtitleFont(dir: File) {
    val target = File(dir, "subfont.ttf")
    if (target.length() > 0) return
    val system = File("/system/fonts")
    val preferred = listOf(
      "Roboto-Regular.ttf",
      "RobotoStatic-Regular.ttf",
      "NotoSans-Regular.ttf",
      "GoogleSans-Regular.ttf",
      "DroidSans.ttf",
    )
    val source = preferred.map { File(system, it) }.firstOrNull { it.canRead() && it.length() > 0 }
      // Any other regular sans that reads Latin text: not a symbol, emoji,
      // monospace or serif face, and not one for a single other script.
      ?: system.listFiles()
        ?.filter { font ->
          val name = font.name
          name.endsWith("-Regular.ttf") && "Sans" in name && font.canRead() &&
            listOf("Symbol", "Emoji", "Mono", "Serif", "CJK").none { it in name } &&
            (name == "NotoSans-Regular.ttf" || !name.startsWith("NotoSans"))
        }
        ?.minByOrNull { it.name }
      ?: return
    try {
      dir.mkdirs()
      val partial = File(dir, "subfont.ttf.part")
      source.copyTo(partial, overwrite = true)
      partial.renameTo(target)
    } catch (e: Exception) {
      // Without it subtitles do not show, but nothing else is affected.
    }
  }

  private fun formatOf(format: String): Int = when (format) {
    "flag" -> MPVLib.MPV_FORMAT_FLAG
    "int64" -> MPVLib.MPV_FORMAT_INT64
    "double" -> MPVLib.MPV_FORMAT_DOUBLE
    "string" -> MPVLib.MPV_FORMAT_STRING
    else -> MPVLib.MPV_FORMAT_NONE
  }

  /** The surface, beneath the page, in the same frame. */
  private fun attachSurface(webView: WebView?) {
    activity.runOnUiThread {
      if (surface != null) return@runOnUiThread
      val page = webView ?: return@runOnUiThread
      val parent = page.parent as? ViewGroup ?: return@runOnUiThread
      val view = SurfaceView(activity)
      view.holder.addCallback(this)
      parent.addView(
        view,
        0,
        FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
      )
      // The page is see-through wherever it draws nothing; it draws its own
      // background everywhere except over a film.
      page.setBackgroundColor(Color.TRANSPARENT)
      surface = view
    }
  }

  override fun surfaceCreated(holder: SurfaceHolder) {
    MPVLib.attachSurface(holder.surface)
    MPVLib.setPropertyString("vo", "gpu")
  }

  override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
    MPVLib.setPropertyString("android-surface-size", "${width}x$height")
  }

  override fun surfaceDestroyed(holder: SurfaceHolder) {
    // In this order, or mpv is left drawing into a surface that is gone. The
    // sound carries on: music with the screen off, or the app in the back.
    MPVLib.setPropertyString("vo", "null")
    MPVLib.detachSurface()
  }

  fun command(args: Array<String>) {
    if (started) MPVLib.command(args)
  }

  fun setProperty(name: String, value: String) {
    if (started) MPVLib.setPropertyString(name, value)
  }

  fun getProperty(name: String, format: String): Any? {
    if (!started) return null
    return try {
      when (format) {
        "flag" -> MPVLib.getPropertyBoolean(name)
        "int64" -> MPVLib.getPropertyInt(name)
        "double" -> MPVLib.getPropertyDouble(name)
        else -> MPVLib.getPropertyString(name)
      }
    } catch (e: Exception) {
      null
    }
  }

  fun destroy() {
    if (!started) return
    MPVLib.removeObserver(this)
    MPVLib.destroy()
    started = false
  }

  // mpv's event thread ----------------------------------------------------

  private fun changed(name: String, data: Any?) {
    // The clock changes every frame. Four times a second is plenty for a
    // scrubber, and spares the bridge sixty messages a second.
    if (name == "time-pos") {
      val now = SystemClock.uptimeMillis()
      if (now - lastTime < 250) return
      lastTime = now
    }
    val event = JSObject().put("event", "property-change").put("name", name)
    if (data == null) event.put("data", JSONObject.NULL) else event.put("data", data)
    emit(event)
  }

  override fun eventProperty(property: String) = changed(property, null)
  override fun eventProperty(property: String, value: Long) = changed(property, value)
  override fun eventProperty(property: String, value: Double) = changed(property, value)
  override fun eventProperty(property: String, value: Boolean) = changed(property, value)
  override fun eventProperty(property: String, value: String) = changed(property, value)
  override fun event(eventId: Int) {}
}
