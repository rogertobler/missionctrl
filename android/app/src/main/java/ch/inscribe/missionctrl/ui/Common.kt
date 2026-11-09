package ch.inscribe.missionctrl.ui

import android.content.Context
import android.text.InputType
import android.text.method.PasswordTransformationMethod
import android.view.KeyEvent
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.res.ResourcesCompat
import androidx.core.widget.doAfterTextChanged
import ch.inscribe.missionctrl.R

@Composable
fun McIcon(icon: ImageVector, tint: Color, size: Dp = 18.dp, description: String? = null) {
    Icon(icon, contentDescription = description, tint = tint, modifier = Modifier.size(size))
}

/** The workspace colour mark: a slim vertical bar. */
@Composable
fun ColorBar(color: Color, height: Dp = 22.dp, width: Dp = 4.dp) {
    Box(Modifier.width(width).height(height).clip(RoundedCornerShape(2.dp)).background(color))
}

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    val c = LocalMc.current
    Text(text.uppercase(), style = McType.section, color = c.muted, modifier = modifier.padding(horizontal = 4.dp))
}

/** Rounded surface block used for grouped rows. */
@Composable
fun Block(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalMc.current
    CompositionLocalProvider(LocalInBlock provides true) {
        Column(modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface), content = content)
    }
}

/** True inside a [Block]: a [SecondaryButton] there takes the inset colour, else it vanishes in the block. */
val LocalInBlock = staticCompositionLocalOf { false }

@Composable
fun Divider() {
    val c = LocalMc.current
    Box(Modifier.fillMaxWidth().height(1.dp).background(c.line))
}

/** Small rounded pill, e.g. "wartet". */
@Composable
fun Pill(text: String, fg: Color, bg: Color, dot: Boolean = false) {
    Row(
        Modifier.height(26.dp).clip(RoundedCornerShape(13.dp)).background(bg).padding(horizontal = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (dot) Box(Modifier.size(7.dp).clip(RoundedCornerShape(4.dp)).background(fg))
        Text(text, style = McType.caption.copy(fontWeight = FontWeight.SemiBold), color = fg, maxLines = 1)
    }
}

/** Icon-only square button, at least 44 dp. */
@Composable
fun IconAction(icon: ImageVector, description: String, tint: Color = LocalMc.current.muted, size: Dp = 44.dp, iconSize: Dp = 20.dp, onClick: () -> Unit) {
    Box(
        Modifier.size(size).clip(RoundedCornerShape(12.dp)).clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) { McIcon(icon, tint, iconSize) }
}

/** Height and icon size of button bars and the answer field (Settings › Schnellknöpfe › Höhe). */
data class KeySize(val height: Dp, val icon: Dp) {
    companion object {
        val NORMAL = KeySize(48.dp, 18.dp)
        val COMPACT = KeySize(40.dp, 16.dp)
    }
}

val LocalKeys = staticCompositionLocalOf { KeySize.NORMAL }

/** One cell of a button bar. Either an icon or a text label. */
data class BarButton(val id: String, val description: String, val icon: ImageVector? = null, val label: String? = null, val onClick: () -> Unit)

/**
 * A flat bar of equal cells separated by 1 dp lines, embedded at the bottom
 * of a card or as the detail toolbar. Neutral: no cell is highlighted.
 */
@Composable
fun ButtonBar(buttons: List<BarButton>, modifier: Modifier = Modifier, wide: Set<String> = emptySet()) {
    val c = LocalMc.current
    val keys = LocalKeys.current
    Row(modifier.fillMaxWidth().background(c.line), horizontalArrangement = Arrangement.spacedBy(1.dp)) {
        buttons.forEach { b ->
            Box(
                Modifier.weight(if (b.id in wide) 1.6f else 1f).height(keys.height).background(c.surface)
                    .clickable(role = Role.Button, onClick = b.onClick).semantics { contentDescription = b.description },
                contentAlignment = Alignment.Center,
            ) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    b.icon?.let { McIcon(it, c.text, if (b.id == "stop") 12.dp else keys.icon) }
                    b.label?.let { Text(it, style = McType.small, color = c.text, maxLines = 1) }
                }
            }
        }
    }
}

/** Segmented control (two or three options). */
@Composable
fun Segmented(options: List<String>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier, compact: Boolean = false) {
    val c = LocalMc.current
    Row(modifier.clip(RoundedCornerShape(10.dp)).background(if (compact) c.surface else c.inset).padding(2.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        options.forEachIndexed { i, label ->
            val sel = i == selected
            Box(
                Modifier.then(if (compact) Modifier else Modifier.weight(1f)).height(if (compact) 30.dp else 36.dp)
                    .clip(RoundedCornerShape(8.dp)).background(if (sel) (if (compact) c.segActive else c.surface) else Color.Transparent)
                    .selectable(sel, role = Role.Tab) { onSelect(i) }.padding(horizontal = 12.dp),
                contentAlignment = Alignment.Center,
            ) { Text(label, style = McType.caption.copy(fontWeight = FontWeight.Medium), color = if (sel) c.text else c.muted, maxLines = 1) }
        }
    }
}

/** Clickable row with title, optional subtitle and trailing content. */
@Composable
fun RowItem(
    title: String,
    subtitle: String? = null,
    icon: ImageVector? = null,
    onClick: (() -> Unit)? = null,
    value: String? = null,
    trailing: @Composable RowScope.() -> Unit = {},
) {
    val c = LocalMc.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 14.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        icon?.let { McIcon(it, c.text) }
        if (value != null && subtitle == null) {
            // The title stays whole, a long [value] is shortened.
            Text(title, style = McType.body, color = c.text, maxLines = 1)
            Text(value, style = McType.small, color = c.muted, maxLines = 1, overflow = TextOverflow.Ellipsis, textAlign = TextAlign.End, modifier = Modifier.weight(1f))
        } else {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(title, style = McType.body, color = c.text)
                subtitle?.let { Text(it, style = McType.caption, color = c.muted) }
            }
            value?.let { Text(it, style = McType.small, color = c.muted) }
        }
        trailing()
    }
}

@Composable
fun ConfirmDialog(title: String, text: String, confirm: String, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    val c = LocalMc.current
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text(title, style = McType.title, color = c.text) },
        text = { Text(text, style = McType.small, color = c.muted) },
        confirmButton = { TextButton(onClick = onConfirm) { Text(confirm, style = McType.body, color = c.danger) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel), style = McType.body, color = c.text) } },
    )
}

/**
 * Text input whose keyboard must not learn or store what is typed
 * (IME_FLAG_NO_PERSONALIZED_LEARNING). Used for answers to agents.
 * [prose]: free text for an agent – up to five lines, Enter starts a new one; the caller sends
 * with its own button.
 * [suggestions]: the keyboard may suggest and correct words (still without learning).
 * [address]: host names, users, paths – a URI keyboard without capitals or spaces after dots.
 */
@Composable
fun PrivateTextField(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    mono: Boolean = false,
    password: Boolean = false,
    singleLine: Boolean = true,
    autoFocus: Boolean = false,
    number: Boolean = false,
    address: Boolean = false,
    prose: Boolean = false,
    suggestions: Boolean = false,
    onFocus: ((Boolean) -> Unit)? = null,
    onSubmit: (() -> Unit)? = null,
) {
    val c = LocalMc.current
    // The EditText is created once; the latest callbacks are read through these.
    val change by rememberUpdatedState(onValueChange)
    val submit by rememberUpdatedState(onSubmit)
    val focus by rememberUpdatedState(onFocus)
    AndroidView(
        modifier = modifier,
        factory = { ctx ->
            EditText(ctx).apply {
                background = null
                setPadding(0, 0, 0, 0)
                val oneLine = singleLine && !prose
                imeOptions = EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or when {
                    // The keyboard shows its Enter key, not Send.
                    prose -> EditorInfo.IME_FLAG_NO_ENTER_ACTION
                    onSubmit != null -> EditorInfo.IME_ACTION_SEND
                    else -> EditorInfo.IME_ACTION_DONE
                }
                // Single line first: setSingleLine replaces the transformation, which would undo
                // the password masking set by inputType.
                isSingleLine = oneLine
                inputType = when {
                    password -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                    number -> InputType.TYPE_CLASS_NUMBER
                    // Host names, users, paths: no capital letter and no space after a dot (Gboard
                    // turned "server.example.com" into "Server. example. com").
                    address -> InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                    else -> InputType.TYPE_CLASS_TEXT or
                        (if (oneLine) 0 else InputType.TYPE_TEXT_FLAG_MULTI_LINE) or
                        (if (suggestions) InputType.TYPE_TEXT_FLAG_AUTO_CORRECT else InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS)
                }
                if (password) transformationMethod = PasswordTransformationMethod.getInstance()
                // Grows to five lines, then scrolls.
                if (prose) { minLines = 1; maxLines = 5 }
                setOnFocusChangeListener { _, has -> focus?.invoke(has) }
                typeface = ResourcesCompat.getFont(ctx, if (mono) R.font.ibm_plex_mono_regular else R.font.ibm_plex_sans)
                textSize = if (mono) 13f else 15f
                hint = placeholder
                setText(value)
                doAfterTextChanged { s ->
                    val text = s?.toString().orEmpty()
                    if (text != (tag as? String)) { tag = text; change(text) }
                }
                // Once per press: an Enter sent as a key event calls this for down and up as well.
                // Not for prose: a keyboard that sends Enter as a key event would send instead of a new line.
                if (!prose) setOnEditorActionListener { _, _, event ->
                    if (event == null || event.action == KeyEvent.ACTION_DOWN) submit?.invoke()
                    submit != null
                }
                if (autoFocus) post {
                    requestFocus()
                    ctx.getSystemService(InputMethodManager::class.java)?.showSoftInput(this, 0)
                }
            }
        },
        update = { edit ->
            edit.setTextColor(c.text.toArgb())
            edit.setHintTextColor(c.muted.toArgb())
            if (edit.text.toString() != value) {
                edit.tag = value
                edit.setText(value)
                edit.setSelection(value.length)
            }
        },
    )
}

/** Closes the soft keyboard. An EditText that leaves the composition does not do this itself. */
@Composable
fun rememberHideKeyboard(): () -> Unit {
    val view = LocalView.current
    return remember(view) {
        { view.context.getSystemService(InputMethodManager::class.java)?.hideSoftInputFromWindow(view.windowToken, 0); Unit }
    }
}

/** Labelled input box. */
@Composable
fun LabeledField(label: String, value: String, onValueChange: (String) -> Unit, placeholder: String = "", mono: Boolean = false, password: Boolean = false, number: Boolean = false, address: Boolean = false) {
    val c = LocalMc.current
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, style = McType.small, color = c.muted)
        Box(
            Modifier.fillMaxWidth().height(44.dp).clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp))
                .background(c.bg).padding(horizontal = 12.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            PrivateTextField(value, onValueChange, placeholder, Modifier.fillMaxWidth(), mono = mono, password = password, number = number, address = address)
        }
    }
}

@Composable
fun PrimaryButton(text: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalMc.current
    Box(
        modifier.height(48.dp).clip(RoundedCornerShape(12.dp)).background(c.text).clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Text(text, style = McType.bodyStrong, color = c.bg) }
}

@Composable
fun SecondaryButton(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = LocalMc.current.text,
    background: Color = if (LocalInBlock.current) LocalMc.current.inset else LocalMc.current.surface,
    onClick: () -> Unit,
) {
    Box(
        // Padding inside: without a width the text would touch the edges.
        modifier.height(48.dp).clip(RoundedCornerShape(12.dp)).background(background).clickable(role = Role.Button, onClick = onClick).padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) { Text(text, style = McType.body, color = color) }
}

@Composable
fun EllipsisText(text: String, style: TextStyle, color: Color, modifier: Modifier = Modifier) {
    Text(text, style = style, color = color, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = modifier)
}

/** "#RRGGBB" of a 0xRRGGBB colour, and back. */
fun hexRgb(rgb: Int) = "#%06X".format(rgb and 0xFFFFFF)
fun parseRgb(s: String): Int? = s.trim().removePrefix("#").takeIf { it.length == 6 }?.toIntOrNull(16)

/** A switch in the app's accent colours. */
@Composable
fun McSwitch(checked: Boolean, onChange: (Boolean) -> Unit) {
    val c = LocalMc.current
    Switch(checked, onChange, colors = SwitchDefaults.colors(checkedTrackColor = c.accent, checkedThumbColor = c.onAccent))
}

/** Short message at the bottom of the screen (errors: their message, or "Error"). */
fun Context.toast(message: String?, long: Boolean = false) =
    Toast.makeText(this, message ?: getString(R.string.common_error), if (long) Toast.LENGTH_LONG else Toast.LENGTH_SHORT).show()

@Composable
fun VSpace(h: Dp) = Spacer(Modifier.height(h))

@Composable
fun MaxWidth(content: @Composable () -> Unit) = Box(Modifier.widthIn(max = 720.dp)) { content() }

/**
 * Long-press menu to move an entry one place up or down (tabs and workspaces, in herdr's order).
 * [position] is (index, count); the ends grey out.
 */
@Composable
fun MoveMenu(expanded: Boolean, title: String, position: Pair<Int, Int>?, onDismiss: () -> Unit, onMove: (Int) -> Unit, extra: @Composable () -> Unit = {}) {
    val c = LocalMc.current
    DropdownMenu(expanded = expanded, onDismissRequest = onDismiss, containerColor = c.surface) {
        Text(title, style = McType.bodyStrong, color = c.text, modifier = Modifier.padding(horizontal = 14.dp, vertical = 8.dp))
        val (i, n) = position ?: (0 to 1)
        DropdownMenuItem(text = { Text(stringResource(R.string.common_move_up), style = McType.body) }, leadingIcon = { McIcon(McIcons.Up, c.text, 16.dp) },
            enabled = i > 0, onClick = { onDismiss(); onMove(-1) })
        DropdownMenuItem(text = { Text(stringResource(R.string.common_move_down), style = McType.body) }, leadingIcon = { McIcon(McIcons.Down, c.text, 16.dp) },
            enabled = i < n - 1, onClick = { onDismiss(); onMove(1) })
        extra()
    }
}
