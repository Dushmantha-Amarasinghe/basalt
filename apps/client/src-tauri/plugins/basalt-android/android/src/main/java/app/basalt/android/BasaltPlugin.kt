package app.basalt.android

import android.Manifest
import android.app.Activity
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.ActivityInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.view.HapticFeedbackConstants
import android.webkit.MimeTypeMap
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class PickArgs {
  /** any, media, image, video, audio, subtitle */
  var kind: String? = null
  var multiple: Boolean? = null
}

@InvokeArg
class UriArgs {
  lateinit var uri: String
}

@InvokeArg
class OpenFdArgs {
  lateinit var uri: String
  var mode: String? = null
}

@InvokeArg
class CreateDownloadArgs {
  lateinit var name: String
  var mime: String? = null
}

@InvokeArg
class FinishDownloadArgs {
  lateinit var uri: String
  var ok: Boolean = false
}

@InvokeArg
class OpenWithArgs {
  lateinit var url: String
  var mime: String? = null
  var title: String? = null
}

@InvokeArg
class OpenDownloadArgs {
  lateinit var uri: String
  var mime: String? = null
}

@InvokeArg
class KeepAliveArgs {
  /** transfer or playback */
  lateinit var reason: String
  var title: String? = null
  var text: String? = null
  /** 0 to 100, or -1 for a moving bar */
  var progress: Int = -1
}

@InvokeArg
class LetGoArgs {
  lateinit var reason: String
}

@InvokeArg
class ImmersiveArgs {
  var on: Boolean = false
}

@InvokeArg
class OrientationArgs {
  /** landscape, portrait or auto */
  lateinit var mode: String
}

@InvokeArg
class InstallArgs {
  lateinit var path: String
}

@InvokeArg
class HapticArgs {
  /** tap, long or confirm */
  var kind: String? = null
}

@InvokeArg
class MpvInitArgs {
  var options: Map<String, String> = emptyMap()
  /** Each property the page wants to hear about, and in what form. */
  var observed: Map<String, String> = emptyMap()
}

@InvokeArg
class MpvCommandArgs {
  var args: Array<String> = emptyArray()
}

@InvokeArg
class MpvSubtitleArgs {
  lateinit var uri: String
  var flag: String? = null
}

@InvokeArg
class MpvPropertyArgs {
  lateinit var name: String
  var value: String? = null
  var format: String? = null
}

/**
 * Everything Basalt needs from Android that a web page cannot do.
 *
 * Files: the phone's pickers hand back `content://` addresses, not paths, so
 * a picked file is opened here and its descriptor handed to Rust, which reads
 * it like any file. Downloads are created through MediaStore, which needs no
 * storage permission, and written the same way.
 *
 * Staying alive: Android stops an app it cannot see. A transfer or a film
 * keeps a foreground service running, with a notification saying why.
 *
 * The network: a phone on Wi-Fi with no internet quietly sends everything
 * over mobile data instead, where the host is not. The process is bound to
 * Wi-Fi whenever there is Wi-Fi, so the host stays reachable.
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = "notifications")
  ]
)
class BasaltPlugin(private val activity: Activity) : Plugin(activity) {
  private var webView: WebView? = null
  private var multicast: WifiManager.MulticastLock? = null
  private var wifiCallback: ConnectivityManager.NetworkCallback? = null
  private val shared = mutableListOf<JSObject>()
  private var lastInsets = JSObject()
  private val player by lazy { MpvPlayer(activity) { event -> trigger("mpv", event) } }

  override fun load(webView: WebView) {
    this.webView = webView
    watchInsets(webView)
    bindToWifi()
    acquireMulticast()
    activity.intent?.let { collectShared(it) }
  }

  override fun onNewIntent(intent: Intent) {
    if (collectShared(intent)) {
      trigger("shared", JSObject().put("count", shared.size))
    }
  }

  override fun onResume() {
    acquireMulticast()
  }

  override fun onPause() {
    // Only while the app is in front: it keeps the Wi-Fi radio awake for
    // broadcasts, which costs battery for nothing once nobody is looking.
    multicast?.let { if (it.isHeld) it.release() }
  }

  override fun onDestroy() {
    runCatching { player.destroy() }
    wifiCallback?.let {
      runCatching { connectivity().unregisterNetworkCallback(it) }
    }
  }

  // ---------------------------------------------------------------------------
  // The screen
  // ---------------------------------------------------------------------------

  /**
   * The app draws edge to edge, under the status bar and the gesture bar, as
   * Android 15 requires. How much of each edge those cover is passed to the
   * page as CSS variables, so it can keep its content clear of them.
   */
  private fun watchInsets(view: WebView) {
    WindowCompat.setDecorFitsSystemWindows(activity.window, false)
    // The app is dark, so the clock and icons over it are light.
    activity.runOnUiThread {
      WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
        isAppearanceLightStatusBars = false
        isAppearanceLightNavigationBars = false
      }
    }
    ViewCompat.setOnApplyWindowInsetsListener(view) { v, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
      )
      val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
      val density = activity.resources.displayMetrics.density
      val top = bars.top / density
      val bottom = bars.bottom / density
      val left = bars.left / density
      val right = bars.right / density
      val keyboard = ime.bottom / density
      lastInsets = JSObject()
        .put("top", top).put("bottom", bottom).put("left", left).put("right", right)
        .put("keyboard", keyboard)
      val js = "(function(s){s.setProperty('--inset-top','${top}px');" +
        "s.setProperty('--inset-bottom','${bottom}px');" +
        "s.setProperty('--inset-left','${left}px');" +
        "s.setProperty('--inset-right','${right}px');" +
        "s.setProperty('--keyboard','${keyboard}px');})(document.documentElement.style)"
      v.post { (v as WebView).evaluateJavascript(js, null) }
      insets
    }
    view.post { ViewCompat.requestApplyInsets(view) }
  }

  @Command
  fun insets(invoke: Invoke) {
    webView?.let { v -> v.post { ViewCompat.requestApplyInsets(v) } }
    invoke.resolve(lastInsets)
  }

  /** Full screen for a film: status and navigation bars away until swiped. */
  @Command
  fun setImmersive(invoke: Invoke) {
    val args = invoke.parseArgs(ImmersiveArgs::class.java)
    activity.runOnUiThread {
      val controller = WindowCompat.getInsetsController(activity.window, activity.window.decorView)
      if (args.on) {
        controller.systemBarsBehavior =
          WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller.hide(WindowInsetsCompat.Type.systemBars())
      } else {
        controller.show(WindowInsetsCompat.Type.systemBars())
      }
      invoke.resolve()
    }
  }

  @Command
  fun setOrientation(invoke: Invoke) {
    val args = invoke.parseArgs(OrientationArgs::class.java)
    activity.runOnUiThread {
      activity.requestedOrientation = when (args.mode) {
        "landscape" -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        "portrait" -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
        else -> ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
      }
      invoke.resolve()
    }
  }

  /** Back at the top level: to the home screen, as every app does, not closed. */
  @Command
  fun minimize(invoke: Invoke) {
    activity.runOnUiThread {
      activity.moveTaskToBack(true)
      invoke.resolve()
    }
  }

  @Command
  fun haptic(invoke: Invoke) {
    val args = invoke.parseArgs(HapticArgs::class.java)
    val view = webView ?: return invoke.resolve()
    view.post {
      view.performHapticFeedback(
        when (args.kind) {
          "long" -> HapticFeedbackConstants.LONG_PRESS
          "confirm" -> if (Build.VERSION.SDK_INT >= 30) HapticFeedbackConstants.CONFIRM
            else HapticFeedbackConstants.VIRTUAL_KEY
          else -> HapticFeedbackConstants.VIRTUAL_KEY
        }
      )
      invoke.resolve()
    }
  }

  // ---------------------------------------------------------------------------
  // Picking files
  // ---------------------------------------------------------------------------

  @Command
  fun pickFiles(invoke: Invoke) {
    val args = invoke.parseArgs(PickArgs::class.java)
    val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      putExtra(Intent.EXTRA_ALLOW_MULTIPLE, args.multiple ?: true)
      when (args.kind) {
        "media" -> {
          type = "*/*"
          putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("image/*", "video/*"))
        }
        "image" -> type = "image/*"
        "video" -> type = "video/*"
        "audio" -> type = "audio/*"
        "subtitle" -> type = "*/*"
        else -> type = "*/*"
      }
    }
    startActivityForResult(invoke, intent, "filesPicked")
  }

  @ActivityCallback
  fun filesPicked(invoke: Invoke, result: ActivityResult) {
    val files = JSArray()
    if (result.resultCode == Activity.RESULT_OK) {
      val data = result.data
      val uris = mutableListOf<Uri>()
      data?.clipData?.let { clip -> for (i in 0 until clip.itemCount) uris.add(clip.getItemAt(i).uri) }
      if (uris.isEmpty()) data?.data?.let { uris.add(it) }
      for (uri in uris) files.put(describe(uri))
    }
    invoke.resolve(JSObject().put("files", files))
  }

  @Command
  fun pickFolder(invoke: Invoke) {
    startActivityForResult(invoke, Intent(Intent.ACTION_OPEN_DOCUMENT_TREE), "folderPicked")
  }

  @ActivityCallback
  fun folderPicked(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    if (result.resultCode != Activity.RESULT_OK || uri == null) {
      invoke.resolve(JSObject())
      return
    }
    val id = DocumentsContract.getTreeDocumentId(uri)
    val name = id.substringAfterLast(':').substringAfterLast('/').ifEmpty { "Folder" }
    invoke.resolve(JSObject().put("uri", uri.toString()).put("name", name))
  }

  /**
   * Every file under a picked folder, with its path inside it — so the
   * folder can be recreated on the drive as it is on the phone.
   */
  @Command
  fun listFolder(invoke: Invoke) {
    val args = invoke.parseArgs(UriArgs::class.java)
    Thread {
      try {
        val tree = Uri.parse(args.uri)
        val files = JSArray()
        val folders = JSArray()
        walk(tree, DocumentsContract.getTreeDocumentId(tree), "", files, folders)
        invoke.resolve(JSObject().put("files", files).put("folders", folders))
      } catch (e: Exception) {
        invoke.reject(e.message ?: "could not read the folder")
      }
    }.start()
  }

  private fun walk(tree: Uri, parentId: String, prefix: String, files: JSArray, folders: JSArray) {
    val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId)
    val columns = arrayOf(
      DocumentsContract.Document.COLUMN_DOCUMENT_ID,
      DocumentsContract.Document.COLUMN_DISPLAY_NAME,
      DocumentsContract.Document.COLUMN_MIME_TYPE,
      DocumentsContract.Document.COLUMN_SIZE,
      DocumentsContract.Document.COLUMN_LAST_MODIFIED,
    )
    activity.contentResolver.query(children, columns, null, null, null)?.use { c ->
      while (c.moveToNext()) {
        val id = c.getString(0)
        val name = c.getString(1) ?: continue
        val mime = c.getString(2) ?: ""
        val rel = if (prefix.isEmpty()) name else "$prefix/$name"
        if (mime == DocumentsContract.Document.MIME_TYPE_DIR) {
          folders.put(rel)
          walk(tree, id, rel, files, folders)
        } else {
          files.put(
            JSObject()
              .put("uri", DocumentsContract.buildDocumentUriUsingTree(tree, id).toString())
              .put("rel", rel)
              .put("name", name)
              .put("size", if (c.isNull(3)) 0L else c.getLong(3))
              .put("mtime", if (c.isNull(4)) 0L else c.getLong(4) / 1000)
          )
        }
      }
    }
  }

  /** A picked or shared file: its name, size and type, as far as it says. */
  private fun describe(uri: Uri): JSObject {
    var name = uri.lastPathSegment ?: "file"
    var size = -1L
    var mtime = 0L
    runCatching {
      activity.contentResolver.query(uri, null, null, null, null)?.use { c ->
        if (c.moveToFirst()) {
          val n = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
          val s = c.getColumnIndex(OpenableColumns.SIZE)
          val m = c.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED)
          if (n >= 0 && !c.isNull(n)) name = c.getString(n)
          if (s >= 0 && !c.isNull(s)) size = c.getLong(s)
          if (m >= 0 && !c.isNull(m)) mtime = c.getLong(m) / 1000
        }
      }
    }
    if (size < 0) {
      // Some sources do not say. The descriptor usually knows.
      runCatching {
        activity.contentResolver.openFileDescriptor(uri, "r")?.use { size = it.statSize }
      }
    }
    val mime = runCatching { activity.contentResolver.getType(uri) }.getOrNull() ?: ""
    // A source that will not say what the file is called leaves only the end
    // of its address, a bare number; it is at least given its type's ending.
    if (!name.contains('.')) {
      MimeTypeMap.getSingleton().getExtensionFromMimeType(mime)?.let { name = "$name.$it" }
    }
    return JSObject()
      .put("uri", uri.toString())
      .put("name", name)
      .put("size", size)
      .put("mtime", mtime)
      .put("mime", mime)
  }

  /** Called from Rust: a picked file, opened, as a descriptor it now owns. */
  @Command
  fun openFd(invoke: Invoke) {
    val args = invoke.parseArgs(OpenFdArgs::class.java)
    try {
      val pfd = activity.contentResolver.openFileDescriptor(Uri.parse(args.uri), args.mode ?: "r")
        ?: return invoke.reject("could not open ${args.uri}")
      invoke.resolve(JSObject().put("fd", pfd.detachFd()))
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not open the file")
    }
  }

  // ---------------------------------------------------------------------------
  // Shared into Basalt
  // ---------------------------------------------------------------------------

  private fun collectShared(intent: Intent): Boolean {
    val uris = mutableListOf<Uri>()
    when (intent.action) {
      Intent.ACTION_SEND -> {
        @Suppress("DEPRECATION")
        (intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM))?.let { uris.add(it) }
      }
      Intent.ACTION_SEND_MULTIPLE -> {
        @Suppress("DEPRECATION")
        intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)?.let { uris.addAll(it) }
      }
      else -> return false
    }
    if (uris.isEmpty()) return false
    for (uri in uris) shared.add(describe(uri))
    // Handled: a second resume must not offer the same files again.
    intent.action = Intent.ACTION_MAIN
    return true
  }

  /** Files shared into the app since it last asked, once each. */
  @Command
  fun takeShared(invoke: Invoke) {
    val files = JSArray()
    for (f in shared) files.put(f)
    shared.clear()
    invoke.resolve(JSObject().put("files", files))
  }

  // ---------------------------------------------------------------------------
  // Downloads
  // ---------------------------------------------------------------------------

  /**
   * A new file in Downloads/Basalt, hidden until it is finished, open for
   * writing. Through MediaStore on Android 10 and later, which needs no
   * permission at all; straight into the folder before that.
   */
  @Command
  fun createDownload(invoke: Invoke) {
    val args = invoke.parseArgs(CreateDownloadArgs::class.java)
    val mime = args.mime?.takeIf { it.isNotEmpty() } ?: mimeOf(args.name)
    try {
      if (Build.VERSION.SDK_INT >= 29) {
        val values = ContentValues().apply {
          put(MediaStore.Downloads.DISPLAY_NAME, args.name)
          put(MediaStore.Downloads.MIME_TYPE, mime)
          put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/Basalt")
          put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = activity.contentResolver.insert(
          MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values
        ) ?: return invoke.reject("Downloads would not take a new file")
        val pfd = activity.contentResolver.openFileDescriptor(uri, "w")
          ?: return invoke.reject("could not open the new download")
        invoke.resolve(
          JSObject()
            .put("uri", uri.toString())
            .put("fd", pfd.detachFd())
            .put("shownAs", "Download/Basalt/${args.name}")
        )
      } else {
        @Suppress("DEPRECATION")
        val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "Basalt")
        dir.mkdirs()
        var file = File(dir, args.name)
        var n = 1
        while (file.exists()) {
          val dot = args.name.lastIndexOf('.')
          val stem = if (dot > 0) args.name.substring(0, dot) else args.name
          val ext = if (dot > 0) args.name.substring(dot) else ""
          file = File(dir, "$stem ($n)$ext")
          n += 1
        }
        val pfd = ParcelFileDescriptor.open(
          file,
          ParcelFileDescriptor.MODE_CREATE or ParcelFileDescriptor.MODE_WRITE_ONLY or
            ParcelFileDescriptor.MODE_TRUNCATE
        )
        invoke.resolve(
          JSObject()
            .put("uri", Uri.fromFile(file).toString())
            .put("fd", pfd.detachFd())
            .put("shownAs", "Download/Basalt/${file.name}")
        )
      }
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not create the download")
    }
  }

  @Command
  fun finishDownload(invoke: Invoke) {
    val args = invoke.parseArgs(FinishDownloadArgs::class.java)
    val uri = Uri.parse(args.uri)
    try {
      if (uri.scheme == "file") {
        if (!args.ok) File(uri.path ?: "").delete()
      } else if (args.ok) {
        val values = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
        activity.contentResolver.update(uri, values, null, null)
      } else {
        activity.contentResolver.delete(uri, null, null)
      }
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject(e.message ?: "could not finish the download")
    }
  }

  /** Opens a finished download in whatever the phone uses for that type. */
  @Command
  fun openDownload(invoke: Invoke) {
    val args = invoke.parseArgs(OpenDownloadArgs::class.java)
    val uri = shareable(Uri.parse(args.uri))
    val intent = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(uri, args.mime?.takeIf { it.isNotEmpty() } ?: activity.contentResolver.getType(uri))
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    start(invoke, Intent.createChooser(intent, null))
  }

  /** Sends a finished download to another app: the share sheet. */
  @Command
  fun shareDownload(invoke: Invoke) {
    val args = invoke.parseArgs(OpenDownloadArgs::class.java)
    val uri = shareable(Uri.parse(args.uri))
    val intent = Intent(Intent.ACTION_SEND).apply {
      type = args.mime?.takeIf { it.isNotEmpty() } ?: activity.contentResolver.getType(uri) ?: "*/*"
      putExtra(Intent.EXTRA_STREAM, uri)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    start(invoke, Intent.createChooser(intent, null))
  }

  /** A file:// address becomes one another app is allowed to open. */
  private fun shareable(uri: Uri): Uri =
    if (uri.scheme == "file") {
      FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", File(uri.path ?: ""))
    } else uri

  // ---------------------------------------------------------------------------
  // Opening in another app
  // ---------------------------------------------------------------------------

  /**
   * A stream from the media proxy, handed to a player that asks for it —
   * VLC, MX Player. They reach it on 127.0.0.1 like this app does.
   */
  @Command
  fun openWith(invoke: Invoke) {
    val args = invoke.parseArgs(OpenWithArgs::class.java)
    val intent = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(Uri.parse(args.url), args.mime?.takeIf { it.isNotEmpty() } ?: "video/*")
      args.title?.let { putExtra("title", it) }
      addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    start(invoke, Intent.createChooser(intent, args.title))
  }

  private fun start(invoke: Invoke, intent: Intent) {
    try {
      activity.startActivity(intent)
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject("No app on this phone can open that.")
    }
  }

  // ---------------------------------------------------------------------------
  // The player
  // ---------------------------------------------------------------------------

  @Command
  fun mpvInit(invoke: Invoke) {
    val args = invoke.parseArgs(MpvInitArgs::class.java)
    try {
      player.init(args.options, args.observed, webView)
      invoke.resolve()
    } catch (e: Throwable) {
      invoke.reject("The player could not start: ${e.message}")
    }
  }

  @Command
  fun mpvCommand(invoke: Invoke) {
    val args = invoke.parseArgs(MpvCommandArgs::class.java)
    player.command(args.args)
    invoke.resolve()
  }

  @Command
  fun mpvSetProperty(invoke: Invoke) {
    val args = invoke.parseArgs(MpvPropertyArgs::class.java)
    player.setProperty(args.name, args.value ?: "")
    invoke.resolve()
  }

  @Command
  fun mpvGetProperty(invoke: Invoke) {
    val args = invoke.parseArgs(MpvPropertyArgs::class.java)
    val value = player.getProperty(args.name, args.format ?: "string")
    invoke.resolve(if (value == null) JSObject() else JSObject().put("value", value))
  }

  /**
   * A subtitle file from the phone's picker. mpv cannot open a `content://`
   * address, so it is given the open file instead; it reads the whole of a
   * subtitle file as it adds it, so the file is closed straight after.
   */
  @Command
  fun mpvAddSubtitle(invoke: Invoke) {
    val args = invoke.parseArgs(MpvSubtitleArgs::class.java)
    val uri = Uri.parse(args.uri)
    try {
      val name = describe(uri).getString("name") ?: "Subtitles"
      val file = activity.contentResolver.openFileDescriptor(uri, "r")
        ?: return invoke.reject("That file could not be opened.")
      file.use { player.command(arrayOf("sub-add", "fd://${it.fd}", args.flag ?: "select", name)) }
      invoke.resolve()
    } catch (e: Exception) {
      invoke.reject("That file could not be opened: ${e.message}")
    }
  }

  @Command
  fun mpvDestroy(invoke: Invoke) {
    player.destroy()
    invoke.resolve()
  }

  // ---------------------------------------------------------------------------
  // Staying awake
  // ---------------------------------------------------------------------------

  @Command
  fun keepAlive(invoke: Invoke) {
    val args = invoke.parseArgs(KeepAliveArgs::class.java)
    KeepAliveService.hold(activity, args.reason, args.title ?: "Basalt", args.text ?: "", args.progress)
    invoke.resolve()
  }

  @Command
  fun letGo(invoke: Invoke) {
    val args = invoke.parseArgs(LetGoArgs::class.java)
    KeepAliveService.release(activity, args.reason)
    invoke.resolve()
  }

  @Command
  fun requestNotifications(invoke: Invoke) {
    if (Build.VERSION.SDK_INT < 33) {
      invoke.resolve(JSObject().put("granted", true))
      return
    }
    requestPermissionForAlias("notifications", invoke, "notificationsAnswered")
  }

  @PermissionCallback
  fun notificationsAnswered(invoke: Invoke) {
    val state = getPermissionState("notifications")
    invoke.resolve(JSObject().put("granted", state.toString() == "granted"))
  }

  // ---------------------------------------------------------------------------
  // Updates
  // ---------------------------------------------------------------------------

  @Command
  fun canInstallApks(invoke: Invoke) {
    val can = Build.VERSION.SDK_INT < 26 || activity.packageManager.canRequestPackageInstalls()
    invoke.resolve(JSObject().put("can", can))
  }

  /** The one-time switch that lets this app install its own updates. */
  @Command
  fun openInstallSettings(invoke: Invoke) {
    if (Build.VERSION.SDK_INT >= 26) {
      val intent = Intent(
        Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
        Uri.parse("package:" + activity.packageName)
      )
      start(invoke, intent)
    } else {
      invoke.resolve()
    }
  }

  /** Hands a verified package to Android's installer, which asks the user. */
  @Command
  fun installApk(invoke: Invoke) {
    val args = invoke.parseArgs(InstallArgs::class.java)
    val file = File(args.path)
    if (!file.exists()) return invoke.reject("the update is not where it was saved")
    val uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
    val intent = Intent(Intent.ACTION_VIEW).apply {
      setDataAndType(uri, "application/vnd.android.package-archive")
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    start(invoke, intent)
  }

  // ---------------------------------------------------------------------------
  // The network
  // ---------------------------------------------------------------------------

  private fun connectivity() =
    activity.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager

  /**
   * Everything this app sends goes over Wi-Fi while there is Wi-Fi.
   *
   * Android prefers mobile data over a Wi-Fi network that has no internet,
   * which a home network with a drive on it may well not have — and then
   * every connection to the host goes out over mobile data and never
   * arrives. When Wi-Fi goes, the binding goes with it.
   */
  private fun bindToWifi() {
    val request = NetworkRequest.Builder()
      .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
      .build()
    val callback = object : ConnectivityManager.NetworkCallback() {
      override fun onAvailable(network: Network) {
        connectivity().bindProcessToNetwork(network)
      }

      override fun onLost(network: Network) {
        if (connectivity().boundNetworkForProcess == network) {
          connectivity().bindProcessToNetwork(null)
        }
      }
    }
    runCatching { connectivity().registerNetworkCallback(request, callback) }
    wifiCallback = callback
  }

  /**
   * Hosts announce themselves by broadcast, and Android drops broadcasts
   * before they reach an app unless it holds this lock.
   */
  private fun acquireMulticast() {
    val wifi = activity.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    val lock = multicast ?: wifi.createMulticastLock("basalt-discovery").also {
      it.setReferenceCounted(false)
      multicast = it
    }
    if (!lock.isHeld) runCatching { lock.acquire() }
  }

  private fun mimeOf(name: String): String {
    val ext = name.substringAfterLast('.', "").lowercase()
    return MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
  }
}
