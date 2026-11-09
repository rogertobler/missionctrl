package ch.inscribe.missionctrl.ui

import android.content.Context
import android.os.Build
import android.view.View
import android.view.ViewTreeObserver
import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.R

/**
 * The sheet behind ⋯: every key the bar does not hold, in groups. Keys keep it open (↓ ↓ ⏎),
 * actions such as speech or a file close it. A long press puts a key into the bar ([onAdd]).
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun AllKeysSheet(available: List<String>, inBar: List<String>, onPress: (QuickKey) -> Unit, onAdd: (QuickKey) -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var sent by remember { mutableStateOf("") }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet, containerColor = c.bg) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 12.dp).padding(bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.allkeys_title), style = McType.title, color = c.text, modifier = Modifier.weight(1f))
                if (sent.isNotEmpty()) Text(stringResource(R.string.allkeys_sent, sent), style = McType.path, color = c.accentText)
            }
            QuickKeys.groups.forEach { (name, ids) ->
                val keys = ids.filter { it in available && it !in inBar }.mapNotNull(QuickKeys::get)
                if (keys.isEmpty()) return@forEach
                SectionTitle(stringResource(name))
                keys.chunked(4).forEach { row ->
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        row.forEach { k ->
                            val label = stringResource(k.label)
                            val description = stringResource(k.description)
                            Column(
                                Modifier.weight(1f).height(56.dp).clip(RoundedCornerShape(12.dp)).background(c.surface)
                                    .combinedClickable(role = Role.Button, onLongClick = { onAdd(k) }) {
                                        if (k.special) onDismiss() else sent = label
                                        onPress(k)
                                    }
                                    .semantics { contentDescription = description },
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(3.dp, Alignment.CenterVertically),
                            ) {
                                if (k.icon != null) McIcon(k.icon, c.text, if (k.id == "stop") 14.dp else 18.dp)
                                else Text(label, style = McType.bodyStrong.copy(fontFamily = McType.mono.fontFamily), color = c.text, maxLines = 1)
                                // "Tab" under "Tab" says nothing: a glyph whose caption repeats it stands alone.
                                val cap = caption(LocalContext.current, k)
                                if (k.icon != null || cap != label) Text(cap, style = McType.caption, color = c.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            }
                        }
                        repeat(4 - row.size) { Box(Modifier.weight(1f)) }
                    }
                }
            }
            Text(stringResource(R.string.allkeys_hint), style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp, vertical = 4.dp))
        }
    }
}

/** Under an icon the name ("Voice"); under a glyph ("⇧Tab", "↑") what it does. */
private fun caption(context: Context, k: QuickKey): String {
    val label = context.getString(k.label)
    return when {
        k.mod != null -> context.getString(R.string.key_next)
        k.icon != null && label.any(Char::isLetter) -> label.removePrefix("⏎ ")
        else -> context.getString(k.description)
    }
}

/** "Ctrl + …" above the bar while a modifier waits for its key; a tap opens the keyboard again, ✕ cancels. */
@Composable
fun ModBar(mod: String, onReopen: () -> Unit, onCancel: () -> Unit) {
    val c = LocalMc.current
    val keys = LocalKeys.current
    Row(
        Modifier.fillMaxWidth().height(keys.height).background(c.accentSoft).clickable(onClick = onReopen).padding(start = 14.dp),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(modLabel(mod) + " + …", style = McType.bodyStrong.copy(fontFamily = McType.mono.fontFamily), color = c.accentText)
        Text(stringResource(R.string.mod_hint), style = McType.caption, color = c.accentText, modifier = Modifier.weight(1f))
        IconAction(McIcons.Close, stringResource(R.string.mod_cancel, modLabel(mod)), c.accentText, size = keys.height, iconSize = 14.dp, onClick = onCancel)
    }
}

fun modLabel(mod: String) = mod.split('+').joinToString("+") { it.replaceFirstChar(Char::uppercase) }

/**
 * While a modifier is armed: an invisible editor opens the keyboard and takes the next key.
 * [onKey] gets its herdr name; nothing is learned, like the TUI's keyboard.
 * A new [focusKey] opens the keyboard again (after it was closed or went elsewhere).
 */
@Composable
fun ModCapture(focusKey: Int, onKey: (String) -> Unit) {
    val latest by rememberUpdatedState(onKey)
    var view by remember { mutableStateOf<TermInputView?>(null) }
    AndroidView(
        modifier = Modifier.size(1.dp),
        factory = { ctx -> TermInputView(ctx).apply { onBytes = { b -> ModKeys.name(String(b))?.let { latest(it) } } }.also { view = it } },
    )
    DisposableEffect(view, focusKey) {
        val stop = view?.let(::showKeyboardWhenFocused)
        onDispose { stop?.invoke() }
    }
}

/**
 * Focus [v] at once and open the keyboard once its window has focus. Right after the ⋯ sheet
 * closes the sheet's window still holds it and a request gets lost; and when the app window gets
 * focus back without a focused editor, Android hides the keyboard (STATE_UNSPECIFIED) – 1 ms after
 * our request (seen on a Galaxy Z Fold8). A second try after 300 ms, if it is still closed. Returns a cleanup.
 */
private fun showKeyboardWhenFocused(v: View): () -> Unit {
    val open = Runnable {
        v.requestFocus()
        v.context.getSystemService(InputMethodManager::class.java)?.showSoftInput(v, 0)
    }
    val retry = Runnable {
        val shown = Build.VERSION.SDK_INT >= 30 && v.rootWindowInsets?.isVisible(WindowInsets.Type.ime()) == true
        if (!shown && v.hasFocus()) open.run()
    }
    fun start() { v.post(open); v.postDelayed(retry, 300) }
    val listener = object : ViewTreeObserver.OnWindowFocusChangeListener {
        override fun onWindowFocusChanged(hasFocus: Boolean) {
            if (!hasFocus) return
            v.viewTreeObserver.removeOnWindowFocusChangeListener(this)
            start()
        }
    }
    v.requestFocus()
    if (v.hasWindowFocus()) start() else v.viewTreeObserver.addOnWindowFocusChangeListener(listener)
    return {
        v.removeCallbacks(open)
        v.removeCallbacks(retry)
        if (v.viewTreeObserver.isAlive) v.viewTreeObserver.removeOnWindowFocusChangeListener(listener)
    }
}

/** Long press in the sheet: the key joins the bar ("start" or "detail"), if there is room. */
fun addToBar(core: Core, which: String, k: QuickKey, context: Context) {
    val max = if (which == "detail") QuickKeys.MAX_DETAIL else QuickKeys.MAX_START
    val bar = core.bar(which)
    if (bar.value.size >= max) context.toast(context.getString(R.string.bar_full), long = true)
    else { bar.set(bar.value + k.id); context.toast(context.getString(R.string.bar_added, caption(context, k))) }
}
