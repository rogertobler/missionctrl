package ch.inscribe.missionctrl.ui

import android.graphics.BitmapFactory
import android.text.format.Formatter
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.ImageRead
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

/** Image files named in pane output, e.g. Claude Code's `Read(/tmp/shot.png)`. */
object ImagePaths {
    // A path that ends in an image extension and does not go on as a longer name (`a.png.txt`,
    // `a.png-old`, `a.png+b`, `dir.png/x`); not part of a URL or a longer path (no `:`, `/` or `%`
    // right before it). Only
    // characters the core accepts; `~` only as `~/` at the start.
    private val path = Regex(
        """(?<![\w:/.~@+%\-])((?:~/|\./|/)?[A-Za-z0-9._+@/\-]*[A-Za-z0-9_+@\-]\.(?:png|jpe?g|webp|gif))(?![\w\-/+@%]|\.\w)""",
        RegexOption.IGNORE_CASE,
    )

    fun find(text: String): List<String> =
        path.findAll(text).map { it.groupValues[1] }.filter { ".." !in it }.distinct().toList()

    /**
     * The terminal broke an image path between [a] and [b]: joined without a space, a path runs
     * across the seam, and [a] does not already end in a whole one. [b] may be several following
     * rows run together, so a path over three rows or more is found. Prose (`client/server` +
     * `model`), a word before a path (`saved to` + `/tmp/…`) and two paths on two rows stay apart.
     */
    fun brokenAcross(a: String, b: String): Boolean {
        if (a.isEmpty() || b.isEmpty() || a.last().isWhitespace() || b.first().isWhitespace()) return false
        val tail = a.takeLast(400)
        if (path.findAll(tail).any { it.groups[1]!!.range.last == tail.length - 1 }) return false
        val glued = tail + b.take(1200)
        // The part before the seam must already be a path ("/tmp/sh", not the word "to" before
        // "/tmp/…"), and the next row must not start a path of its own.
        val freshPath = b.startsWith("/") || b.startsWith("~/") || b.startsWith("./") ||
            // A whole file name ends row a and the next row starts a path: a listing, one per row.
            (fileEnd.containsMatchIn(tail) && '/' in b.substringBefore(' '))
        return !freshPath && path.findAll(glued).any { val r = it.groups[1]!!.range; r.first < tail.length && r.last >= tail.length && '/' in tail.substring(r.first) }
    }

    private val fileEnd = Regex("""/[^\s/]*\.[A-Za-z0-9]{1,5}$""")

    /** [path] as the same file is named elsewhere: relative ones against [cwd] (`./a.png` = `/dir/a.png`). */
    fun resolve(path: String, cwd: String?): String =
        if (isRelative(path) && cwd != null) cwd.trimEnd('/') + "/" + path.removePrefix("./") else path

    /** Relative paths depend on the pane's directory and are often false hits (a name in prose). */
    fun isRelative(path: String) = !path.startsWith("/") && !path.startsWith("~/")

    /** Display line index → paths to preview under it; each path only at the line that names it first (index 0 is newest). */
    fun anchors(lines: List<DisplayLine>, cwd: String? = null): Map<Int, List<String>> {
        val seen = HashSet<String>()
        val out = HashMap<Int, List<String>>()
        val found = lines.map { find(it.text.text) }
        // A file named with its full path somewhere shows there, not under a relative mention
        // (`./a.png` in the prompt that asked for it).
        val named = found.flatten().filterNot(::isRelative).toSet()
        // Oldest line first (index 0 is the newest): the picture shows where it is first named,
        // so it is in view when reading down to it.
        for (i in lines.indices.reversed()) {
            val fresh = found[i].filter { !(isRelative(it) && resolve(it, cwd) in named) && seen.add(resolve(it, cwd)) }
            if (fresh.isNotEmpty()) out[i] = fresh
        }
        return out
    }
}

/** What the app knows about one image on a server. */
sealed interface ImageState {
    data object Loading : ImageState
    data class Loaded(val bitmap: ImageBitmap, val bytes: Int) : ImageState
    data object Missing : ImageState
    data class TooLarge(val size: Long) : ImageState
    data object Failed : ImageState
}

/**
 * Fetched images. Loads run in a scope of their own, one at a time per image and two at most
 * at once, so the history view's polling and scrolling neither cancel nor repeat them.
 */
object ImageCache {
    private class Entry(val state: ImageState, val stamp: String?, val at: Long)
    /** Bounded by the decoded size; other states count as one kilobyte. */
    private val entries = object : LruCache<String, Entry>(48 * 1024) {
        override fun sizeOf(key: String, value: Entry) = (((value.state as? ImageState.Loaded)?.bytes ?: 0) + 1023) / 1024 + 1
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val running = HashMap<String, Deferred<ImageState>>()
    private val slots = Semaphore(2)
    /**
     * After this the server is asked whether the file changed (a new screenshot at the same
     * path); the answer is a few bytes unless it did.
     */
    private const val FRESH_MS = 5_000L
    /** A failed fetch (connection, timeout) is tried again sooner. */
    private const val RETRY_MS = 10_000L
    /** Longest side of a decoded image; enough for the full-screen view, bounded in memory. */
    private const val MAX_SIDE = 2048

    fun cached(key: String): ImageState? = entries.get(key)?.state

    fun key(serverId: String, path: String, cwd: String?) = "$serverId|${if (ImagePaths.isRelative(path)) cwd else ""}|$path"

    suspend fun load(core: Core, serverId: String, path: String, cwd: String?): ImageState {
        val key = key(serverId, path, cwd)
        val job = synchronized(running) {
            // Under the lock, so a fetch that just finished is seen and not repeated.
            val old = entries.get(key)
            val now = System.currentTimeMillis()
            if (old != null && now - old.at < if (old.state == ImageState.Failed) RETRY_MS else FRESH_MS) return old.state
            running.getOrPut(key) {
                scope.async {
                    try { slots.withPermit { fetch(core, serverId, path, cwd, key, old) } }
                    finally { synchronized(running) { running.remove(key) } }
                }
            }
        }
        return job.await()
    }

    private suspend fun fetch(core: Core, serverId: String, path: String, cwd: String?, key: String, old: Entry?): ImageState {
        // Also for a file that did not decode: unchanged, it is not fetched again.
        val known = old?.stamp?.takeIf { it.isNotEmpty() }
        val (state, stamp) = runCatching {
            when (val r = core.readImage(serverId, path, cwd, known)) {
                is ImageRead.Data -> (decode(r.data) ?: ImageState.Failed) to r.stamp
                ImageRead.Unchanged -> old!!.state to known
                ImageRead.Missing -> ImageState.Missing to null
                is ImageRead.TooLarge -> ImageState.TooLarge(r.size.toLong()) to null
            }
        }.getOrElse { (old?.state?.takeIf { it is ImageState.Loaded } ?: ImageState.Failed) to old?.stamp }
        entries.put(key, Entry(state, stamp, System.currentTimeMillis()))
        return state
    }

    private fun decode(data: ByteArray): ImageState.Loaded? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / sample > MAX_SIDE) sample *= 2
        val bitmap = BitmapFactory.decodeByteArray(data, 0, data.size, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
        return ImageState.Loaded(bitmap.asImageBitmap(), bitmap.byteCount)
    }
}

/** How often a shown preview asks whether its file changed (a few bytes unless it did). */
internal const val RECHECK_MS = 10_000L

/**
 * Preview of an image named in a pane, under its line; tap to open it full screen. A relative
 * path that is not there shows nothing: often it was a name in prose, not a file.
 */
@Composable
fun ImagePreview(core: Core, serverId: String, path: String, cwd: String?, showPath: Boolean = false, onOpen: (ImageBitmap) -> Unit) {
    if (ImagePaths.isRelative(path) && cwd == null) return
    val c = LocalMc.current
    val context = LocalContext.current
    val cacheKey = ImageCache.key(serverId, path, cwd)
    // Keyed by the whole identity: another server or directory must not show this image meanwhile.
    var state by remember(cacheKey) { mutableStateOf(ImageCache.cached(cacheKey) ?: ImageState.Loading) }
    LaunchedEffect(cacheKey) {
        // Again now and then while shown: the agent may write a new image to the same path.
        while (true) {
            core.awaitVisible() // quiet while the app is in the background
            state = ImageCache.load(core, serverId, path, cwd)
            delay(RECHECK_MS)
        }
    }
    val quiet = ImagePaths.isRelative(path) && (state == ImageState.Missing || state == ImageState.Failed)
    if (quiet) return
    val name = path.substringAfterLast('/')
    val note = c.muted
    DisableSelection {
        Column(Modifier.padding(top = 4.dp, bottom = 8.dp)) {
            if (showPath) Text(path, style = McType.path, color = note, modifier = Modifier.padding(top = 8.dp, bottom = 4.dp))
            when (val s = state) {
                is ImageState.Loaded -> {
                    val desc = stringResource(R.string.image_open_desc, name)
                    // Sized to the picture itself (at most the width and 240 dp high), so the
                    // rounded corners sit on the picture and not on the row around it.
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        val ratio = s.bitmap.width.toFloat() / s.bitmap.height
                        // Never larger than the picture's own pixels.
                        val own = with(LocalDensity.current) { s.bitmap.width.toDp() }
                        val w = minOf(maxWidth, 240.dp * ratio, own)
                        Image(
                            s.bitmap, contentDescription = null, contentScale = ContentScale.Fit,
                            modifier = Modifier.size(w, w / ratio).clip(RoundedCornerShape(8.dp))
                                .semantics { contentDescription = desc }.clickable { onOpen(s.bitmap) },
                        )
                    }
                }
                ImageState.Loading -> Text(stringResource(R.string.image_loading), style = McType.small, color = note)
                ImageState.Missing -> Text(stringResource(R.string.image_missing), style = McType.small, color = note)
                is ImageState.TooLarge -> Text(stringResource(R.string.image_too_large, Formatter.formatShortFileSize(context, s.size)), style = McType.small, color = note)
                ImageState.Failed -> Text(stringResource(R.string.image_failed), style = McType.small, color = note)
            }
        }
    }
}

/** An image full screen: pinch to zoom, drag to move, double tap to fit again. */
@Composable
fun ImageViewer(bitmap: ImageBitmap, path: String, onDismiss: () -> Unit) {
    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    // The picture as fitted to the screen at zoom 1; panning stops at its edges.
    val fit = if (size.width > 0 && size.height > 0) minOf(size.width.toFloat() / bitmap.width, size.height.toFloat() / bitmap.height) else 1f
    val fitW = bitmap.width * fit
    val fitH = bitmap.height * fit
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black).onSizeChanged { size = it }) {
            Image(
                bitmap, contentDescription = path, contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize()
                    .pointerInput(fitW, fitH) {
                        detectTransformGestures { _, pan, zoom, _ ->
                            scale = (scale * zoom).coerceIn(1f, 8f)
                            val maxX = ((fitW * scale - size.width) / 2).coerceAtLeast(0f)
                            val maxY = ((fitH * scale - size.height) / 2).coerceAtLeast(0f)
                            offset = Offset((offset.x + pan.x).coerceIn(-maxX, maxX), (offset.y + pan.y).coerceIn(-maxY, maxY))
                        }
                    }
                    .pointerInput(Unit) { detectTapGestures(onDoubleTap = { scale = 1f; offset = Offset.Zero }) }
                    .graphicsLayer { scaleX = scale; scaleY = scale; translationX = offset.x; translationY = offset.y },
            )
            Text(
                path, style = McType.path, color = Color.White.copy(alpha = 0.7f),
                modifier = Modifier.align(Alignment.BottomStart).navigationBarsPadding().padding(16.dp),
            )
            Box(
                Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(8.dp).size(48.dp)
                    .clip(RoundedCornerShape(24.dp)).background(Color.Black.copy(alpha = 0.5f)).clickable(onClick = onDismiss),
                contentAlignment = Alignment.Center,
            ) { McIcon(McIcons.Close, Color.White, 22.dp, stringResource(R.string.image_viewer_close)) }
        }
    }
}
