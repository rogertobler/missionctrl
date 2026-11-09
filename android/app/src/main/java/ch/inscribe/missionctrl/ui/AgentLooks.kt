package ch.inscribe.missionctrl.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.TextUnitType
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.Core
import ch.inscribe.missionctrl.Look
import ch.inscribe.missionctrl.PathPrefs
import ch.inscribe.missionctrl.R
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.DirInfo
import ch.inscribe.missionctrl.core.FleetState
import java.text.BreakIterator

// -- Path line ------------------------------------------------------------

object PathLines {
    /** Value of one field for [a], or null when there is nothing to show. */
    fun field(id: String, a: AgentRow, info: DirInfo?, home: String?): String? = when (id) {
        "server" -> a.serverName
        "workspace" -> a.workspace
        "tab" -> a.tab
        "branch" -> info?.branch
        "dir" -> a.cwd?.trimEnd('/')?.substringAfterLast('/')?.ifEmpty { "/" }
        "path" -> a.cwd?.let { tilde(it, home) }
        "repo" -> info?.repo
        "agent" -> a.agentType ?: a.name
        "title" -> a.title
        else -> null
    }?.takeIf { it.isNotBlank() }

    fun tilde(path: String, home: String?): String {
        if (home.isNullOrEmpty() || home == "/") return path
        val h = home.trimEnd('/')
        return when {
            path == h -> "~"
            path.startsWith("$h/") -> "~" + path.removePrefix(h)
            else -> path
        }
    }

    fun join(parts: List<String>, sep: String) = parts.joinToString(if (sep == "/") "/" else " $sep ")

    fun format(a: AgentRow, prefs: PathPrefs, info: DirInfo?, home: String?): String =
        join(prefs.order.filter { it in prefs.on }.mapNotNull { field(it, a, info, home) }, prefs.sep)
}

/** The configured path line for [a]; recomposes when the settings or Git facts change. */
@Composable
fun pathLineOf(core: Core, a: AgentRow): String {
    val prefs by core.pathPrefs.collectAsState()
    val dirs by core.dirs.collectAsState()
    val homes by core.homes.collectAsState()
    return PathLines.format(a, prefs, a.cwd?.let { dirs["${a.serverId}|$it"] }, homes[a.serverId])
}

private val examples = mapOf(
    "server" to "devbox", "workspace" to "Webshop", "tab" to "API", "branch" to "main", "dir" to "webshop", "path" to "~/src/webshop",
    "repo" to "webshop-app", "agent" to "claude", "title" to "Migration 0042",
)

/** Name of a path line field in the settings. */
@Composable
private fun pathFieldLabel(id: String): String = when (id) {
    "server" -> stringResource(R.string.agentlooks_field_server)
    "workspace" -> stringResource(R.string.agentlooks_field_workspace)
    "tab" -> stringResource(R.string.agentlooks_field_tab)
    "branch" -> stringResource(R.string.agentlooks_field_branch)
    "dir" -> stringResource(R.string.agentlooks_field_dir)
    "path" -> stringResource(R.string.agentlooks_field_path)
    "repo" -> stringResource(R.string.agentlooks_field_repo)
    "agent" -> stringResource(R.string.agentlooks_agent_type)
    "title" -> stringResource(R.string.agentlooks_field_title)
    else -> id
}

/** Settings block: fields to tick and order, separator, live preview with a real agent. */
@Composable
fun PathLineSettings(core: Core) {
    val c = LocalMc.current
    val prefs by core.pathPrefs.collectAsState()
    val state by core.state.collectAsState()
    val dirs by core.dirs.collectAsState()
    val homes by core.homes.collectAsState()
    val sample = state.agents.firstOrNull()
    fun value(id: String): String =
        sample?.let { PathLines.field(id, it, it.cwd?.let { d -> dirs["${it.serverId}|$d"] }, homes[it.serverId]) } ?: examples[id].orEmpty()
    val preview = PathLines.join(prefs.order.filter { it in prefs.on }.map(::value).filter { it.isNotEmpty() }, prefs.sep)

    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionTitle(stringResource(R.string.agentlooks_path_line_title))
        Block {
            Row(Modifier.fillMaxWidth().background(c.inset).padding(horizontal = 14.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                if (sample != null) WorkspaceMark(core, sample.serverId, sample.workspaceId, sample.workspace ?: "", tab = sample.tab) else ColorBar(c.accent)
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    EllipsisText(sample?.let { agentTitleOf(core, it) } ?: "API", McType.body, c.text)
                    EllipsisText(preview.ifEmpty { stringResource(R.string.agentlooks_nothing_chosen) }, McType.path, c.muted)
                }
                Text(stringResource(R.string.agentlooks_preview), style = McType.caption, color = c.muted)
            }
            prefs.order.forEachIndexed { i, id ->
                val label = pathFieldLabel(id)
                Divider()
                Row(Modifier.fillMaxWidth().heightIn(min = 52.dp), verticalAlignment = Alignment.CenterVertically) {
                    val on = id in prefs.on
                    Row(
                        Modifier.weight(1f).heightIn(min = 52.dp).clickable(role = Role.Checkbox) { core.pathPrefs.set(prefs.copy(on = if (on) prefs.on - id else prefs.on + id)) }.padding(start = 4.dp),
                        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Checkbox(on, null, colors = CheckboxDefaults.colors(checkedColor = c.accent, checkmarkColor = c.onAccent))
                        Text(label, style = McType.body, color = c.text, modifier = Modifier.weight(1f))
                        EllipsisText(value(id).ifEmpty { "–" }, McType.path, c.muted, Modifier.padding(end = 4.dp))
                    }
                    IconAction(McIcons.Up, stringResource(R.string.agentlooks_field_move_up, label), if (i > 0) c.muted else c.line, size = 40.dp, iconSize = 16.dp) {
                        if (i > 0) core.pathPrefs.set(prefs.copy(order = prefs.order.toMutableList().apply { add(i - 1, removeAt(i)) }))
                    }
                    IconAction(McIcons.Down, stringResource(R.string.agentlooks_field_move_down, label), if (i < prefs.order.lastIndex) c.muted else c.line, size = 40.dp, iconSize = 16.dp) {
                        if (i < prefs.order.lastIndex) core.pathPrefs.set(prefs.copy(order = prefs.order.toMutableList().apply { add(i + 1, removeAt(i)) }))
                    }
                }
            }
            Divider()
            Row(Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Text(stringResource(R.string.agentlooks_separator), style = McType.body, color = c.text, modifier = Modifier.weight(1f))
                PathPrefs.SEPARATORS.forEach { s ->
                    val sel = s == prefs.sep
                    val description = stringResource(R.string.agentlooks_separator_desc, s)
                    Box(
                        Modifier.size(40.dp).clip(RoundedCornerShape(10.dp)).background(if (sel) c.inset else Color.Transparent)
                            .then(if (sel) Modifier.border(1.dp, c.text, RoundedCornerShape(10.dp)) else Modifier)
                            .clickable(role = Role.RadioButton) { core.pathPrefs.set(prefs.copy(sep = s)) }.semantics { contentDescription = description },
                        contentAlignment = Alignment.Center,
                    ) { Text(s, style = McType.mono, color = if (sel) c.text else c.muted) }
                }
            }
            Text(stringResource(R.string.agentlooks_git_hint),
                style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp))
        }
    }
}

// -- Names in lists ---------------------------------------------------------------

/**
 * Lists name an agent by its tab. When another tab is called the same,
 * the workspace follows: "API · Webshop".
 */
object Names {
    private fun norm(s: String) = s.trim().lowercase()

    /** Tab labels that occur more than once across all servers. */
    fun duplicateTabs(state: FleetState): Set<String> =
        state.workspaces.flatMap { ws -> ws.tabs.map { norm(it.label) } }.groupingBy { it }.eachCount().filterValues { it > 1 }.keys

    fun title(tab: String?, workspace: String?, fallback: String, duplicates: Set<String>): String {
        val t = tab?.takeIf { it.isNotBlank() } ?: return workspace ?: fallback
        return if (workspace != null && norm(t) in duplicates) "$t · $workspace" else t
    }
}

@Composable
fun agentTitleOf(core: Core, a: AgentRow): String = tabTitleOf(core, a.tab, a.workspace, a.name)

@Composable
fun tabTitleOf(core: Core, tab: String?, workspace: String?, fallback: String): String {
    val duplicates by core.duplicateTabs.collectAsState()
    return Names.title(tab, workspace, fallback, duplicates)
}

// -- Symbols: predefined or emoji ------------------------------------------------------

/** Predefined symbols: the two agent glyphs, then the workspace symbols. */
val AgentSymbols: Map<String, ImageVector> = linkedMapOf("claude" to McIcons.Claude, "codex" to McIcons.Codex) + McIcons.workspaceSymbols

/** What screen readers say for a predefined symbol (its id stays the stored value). */
@Composable
private fun symbolName(id: String): String = when (id) {
    "claude" -> stringResource(R.string.symbol_claude)
    "codex" -> stringResource(R.string.symbol_codex)
    "chart" -> stringResource(R.string.symbol_chart)
    "code" -> stringResource(R.string.symbol_code)
    "layers" -> stringResource(R.string.symbol_layers)
    "database" -> stringResource(R.string.symbol_database)
    "globe" -> stringResource(R.string.symbol_globe)
    "rocket" -> stringResource(R.string.symbol_rocket)
    "wrench" -> stringResource(R.string.symbol_wrench)
    else -> id
}

/** A look's symbol is a predefined id or an emoji, stored as "emoji:🚀". */
object Symbols {
    const val EMOJI = "emoji:"
    val presets = listOf("🚀", "🧪", "📊", "🐛", "🔥", "⭐", "📝", "🛠️", "💡", "🎯", "🧠", "📦", "🌐", "🔒", "💬", "🤖")

    fun emoji(icon: String?): String? = icon?.takeIf { it.startsWith(EMOJI) }?.removePrefix(EMOJI)?.ifEmpty { null }

    fun exists(icon: String?): Boolean = emoji(icon) != null || (icon != null && icon in AgentSymbols)

    /** First user-perceived character of [text] (an emoji may be several code points). */
    fun first(text: String): String? {
        val t = text.trim()
        if (t.isEmpty()) return null
        val it = BreakIterator.getCharacterInstance().apply { setText(t) }
        return t.substring(0, it.next().takeIf { n -> n > 0 } ?: t.length)
    }
}

/** Draws [icon] (predefined or emoji); [fallback] when there is none. Nothing if both are missing. */
@Composable
fun SymbolMark(icon: String?, tint: Color, size: Dp, fallback: ImageVector? = null, description: String? = null) {
    val emoji = Symbols.emoji(icon)
    val vector = icon?.let { AgentSymbols[it] } ?: if (emoji == null) fallback else null
    when {
        emoji != null -> Box(Modifier.size(size + 2.dp).semantics { description?.let { contentDescription = it } }, contentAlignment = Alignment.Center) {
            Text(emoji, fontSize = TextUnit(size.value * 0.9f, TextUnitType.Sp), lineHeight = TextUnit(size.value, TextUnitType.Sp))
        }
        vector != null -> McIcon(vector, tint, size, description)
    }
}

/**
 * Choose a symbol: none, a predefined one, an emoji from a short list, or any emoji typed with
 * the keyboard's emoji page.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun SymbolPicker(selected: String?, tint: Color, noneLabel: String, onPick: (String?) -> Unit) {
    val c = LocalMc.current
    var draft by remember { mutableStateOf(Symbols.emoji(selected)?.takeIf { it !in Symbols.presets } ?: "") }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            LookCell(selected == null, noneLabel, Modifier.size(44.dp), onClick = { onPick(null) }) { McIcon(McIcons.Close, c.muted, 14.dp) }
            AgentSymbols.forEach { (id, icon) ->
                LookCell(selected == id, symbolName(id), Modifier.size(44.dp), onClick = { onPick(id) }) { McIcon(icon, if (selected == id) tint else c.muted, 18.dp) }
            }
        }
        Text(stringResource(R.string.agentlooks_emoji), style = McType.small, color = c.text)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Symbols.presets.forEach { e ->
                val id = Symbols.EMOJI + e
                LookCell(selected == id, e, Modifier.size(44.dp), onClick = { onPick(id) }) { Text(e, fontSize = TextUnit(18f, TextUnitType.Sp)) }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(
                Modifier.weight(1f).height(44.dp).clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp)).background(c.bg).padding(horizontal = 12.dp),
                contentAlignment = Alignment.CenterStart,
            ) { PrivateTextField(draft, { draft = it }, stringResource(R.string.agentlooks_own_emoji_hint), Modifier.fillMaxWidth(), onSubmit = { Symbols.first(draft)?.let { onPick(Symbols.EMOJI + it) } }) }
            SecondaryButton(stringResource(R.string.agentlooks_take)) { Symbols.first(draft)?.let { draft = it; onPick(Symbols.EMOJI + it) } }
        }
    }
}

/**
 * Colour mark of a row: the workspace's symbol, else a bar. Colour: the tab's own ([tab]), else
 * the workspace's, else a faint grey.
 */
@Composable
fun WorkspaceMark(core: Core, serverId: String, workspaceId: String, label: String, height: Dp = 22.dp, tab: String? = null) {
    val looks by core.looks.collectAsState()
    val color = looks.paneColorOrNull(serverId, workspaceId, label, tab) ?: LookBook.NO_COLOR_GREY
    val icon = looks.workspaceSymbol(serverId, workspaceId, label)?.takeIf { Symbols.exists(it) }
    if (icon != null) SymbolMark(icon, color, (height.value * 0.75f).coerceIn(14f, 22f).dp, description = label) else ColorBar(color, height)
}

/** Whose colour a [LookFields] edits; decides what "no colour" means. */
enum class ColorOwner { WORKSPACE, TAB, AGENT }

/**
 * Colour and symbol of a look. Workspace: automatic (from the name), none, or a colour. Tab: like
 * its workspace ([inherited]) or its own. Agent: standard (text colour) or a colour.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun LookFields(look: Look, owner: ColorOwner, noneSymbol: String, autoColor: Color? = null, inherited: Color? = null, onChange: (Look) -> Unit) {
    val c = LocalMc.current
    val chosen = look.ownColor()
    var custom by remember { mutableStateOf(false) }
    val plain = look.rgb == null
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(stringResource(R.string.agentlooks_color), style = McType.small, color = c.text)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            @Composable
            fun word(text: String, selected: Boolean, description: String, value: Int?) =
                LookCell(selected, description, Modifier.height(40.dp), onClick = { onChange(look.copy(color = value, rgb = null)) }) {
                    Text(text, style = McType.caption, color = if (selected) c.text else c.muted, modifier = Modifier.padding(horizontal = 10.dp))
                }
            when (owner) {
                ColorOwner.WORKSPACE -> {
                    word(stringResource(R.string.agentlooks_color_auto), plain && look.color == null, stringResource(R.string.agentlooks_color_auto_desc), null)
                    word(stringResource(R.string.agentlooks_color_none), plain && look.color == Look.NO_COLOR, stringResource(R.string.agentlooks_color_none_desc), Look.NO_COLOR)
                }
                ColorOwner.TAB -> word(stringResource(R.string.agentlooks_color_like_workspace), plain && look.color == null, stringResource(R.string.agentlooks_color_like_workspace_desc), null)
                ColorOwner.AGENT -> word(stringResource(R.string.agentlooks_default), plain && look.color == null, stringResource(R.string.agentlooks_color_default_desc), null)
            }
            WorkspacePalette.forEachIndexed { i, col ->
                LookCell(plain && look.color == i, stringResource(R.string.agentlooks_color_n, i + 1), Modifier.size(40.dp), onClick = { onChange(look.copy(color = i, rgb = null)) }) {
                    Box(Modifier.size(18.dp).clip(RoundedCornerShape(9.dp)).background(col))
                }
            }
            // Any colour: hue, saturation, brightness or a hex code.
            LookCell(!plain, stringResource(R.string.agentlooks_custom_color), Modifier.height(40.dp), onClick = { custom = true }) {
                Row(Modifier.padding(horizontal = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Box(Modifier.size(18.dp).clip(RoundedCornerShape(9.dp)).background(
                        look.rgb?.let(::rgbColor) ?: Color.Transparent,
                    ).then(if (look.rgb == null) Modifier.border(1.5.dp, c.muted, RoundedCornerShape(9.dp)) else Modifier))
                    Text(stringResource(R.string.agentlooks_custom), style = McType.caption, color = if (!plain) c.text else c.muted)
                }
            }
        }
        if (custom) ColorDialog(look.rgb ?: chosen?.let(::rgbOf) ?: autoColor?.let(::rgbOf) ?: 0x00E28A, onDismiss = { custom = false }) {
            custom = false; onChange(look.copy(rgb = it))
        }
        Text(
            when (owner) {
                ColorOwner.WORKSPACE -> if (!plain) stringResource(R.string.agentlooks_workspace_custom_hint) else when (look.color) {
                    null -> stringResource(R.string.agentlooks_workspace_auto_hint)
                    Look.NO_COLOR -> stringResource(R.string.agentlooks_workspace_none_hint)
                    else -> stringResource(R.string.agentlooks_workspace_palette_hint)
                }
                ColorOwner.TAB -> if (plain && look.color == null) {
                    if (inherited == null) stringResource(R.string.agentlooks_tab_inherit_none_hint) else stringResource(R.string.agentlooks_tab_inherit_hint)
                } else stringResource(R.string.agentlooks_tab_custom_hint)
                ColorOwner.AGENT -> if (plain && look.color == null) stringResource(R.string.agentlooks_agent_default_hint) else stringResource(R.string.agentlooks_agent_color_hint)
            },
            style = McType.caption, color = c.muted,
        )
        Text(stringResource(R.string.agentlooks_icon), style = McType.small, color = c.text)
        val tint = chosen ?: (if (owner == ColorOwner.WORKSPACE && look.color == null) autoColor else inherited) ?: c.text
        SymbolPicker(look.icon, tint, noneSymbol) { onChange(look.copy(icon = it)) }
    }
}

fun rgbOf(color: Color): Int = color.toArgb() and 0xFFFFFF

/** Free colour choice: hue, saturation and brightness sliders, or a hex code like #3A7BD5. */
@Composable
fun ColorDialog(initial: Int, onDismiss: () -> Unit, onPick: (Int) -> Unit) {
    val c = LocalMc.current
    val start = remember { FloatArray(3).also { android.graphics.Color.colorToHSV(0xFF000000.toInt() or initial, it) } }
    var hue by remember { mutableFloatStateOf(start[0]) }
    var sat by remember { mutableFloatStateOf(start[1]) }
    var value by remember { mutableFloatStateOf(start[2]) }
    val rgb = android.graphics.Color.HSVToColor(floatArrayOf(hue, sat, value)) and 0xFFFFFF
    var hex by remember { mutableStateOf(hexRgb(initial)) }
    LaunchedEffect(rgb) { if (parseRgb(hex) != rgb) hex = hexRgb(rgb) }
    val sliderColors = SliderDefaults.colors(thumbColor = c.text, activeTrackColor = c.text, inactiveTrackColor = c.line)
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text(stringResource(R.string.agentlooks_custom_color), style = McType.title, color = c.text) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    ColorBar(rgbColor(rgb), 40.dp, 6.dp)
                    Box(Modifier.size(40.dp).clip(RoundedCornerShape(20.dp)).background(rgbColor(rgb)))
                    Box(
                        Modifier.weight(1f).height(44.dp).clip(RoundedCornerShape(10.dp)).border(1.dp, c.line, RoundedCornerShape(10.dp)).background(c.bg).padding(horizontal = 12.dp),
                        contentAlignment = Alignment.CenterStart,
                    ) {
                        PrivateTextField(hex, { t ->
                            hex = t
                            parseRgb(t)?.let { v -> FloatArray(3).also { android.graphics.Color.colorToHSV(0xFF000000.toInt() or v, it) }.let { hue = it[0]; sat = it[1]; value = it[2] } }
                        }, "#RRGGBB", Modifier.fillMaxWidth(), mono = true)
                    }
                }
                Text(stringResource(R.string.agentlooks_hue), style = McType.caption, color = c.muted)
                // Hue strip under the slider, so the position is obvious.
                Box(Modifier.fillMaxWidth().height(8.dp).clip(RoundedCornerShape(4.dp)).background(
                    Brush.horizontalGradient((0..6).map { rgbColor(android.graphics.Color.HSVToColor(floatArrayOf(it * 60f, 1f, 1f))) }),
                ))
                Slider(hue, { hue = it }, valueRange = 0f..360f, colors = sliderColors)
                Text(stringResource(R.string.agentlooks_saturation), style = McType.caption, color = c.muted)
                Slider(sat, { sat = it }, colors = sliderColors)
                Text(stringResource(R.string.agentlooks_brightness), style = McType.caption, color = c.muted)
                Slider(value, { value = it }, colors = sliderColors)
            }
        },
        confirmButton = { TextButton(onClick = { onPick(rgb) }) { Text(stringResource(R.string.agentlooks_take), style = McType.body, color = c.text) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.common_cancel), style = McType.body, color = c.muted) } },
    )
}


// -- Agent symbol and looks -----------------------------------------------------

/**
 * The agent's symbol (✳ Claude, ◆ Codex or its own), tinted with its own colour.
 * Setting "Agent icon": automatic shows it only when different agent types are around or
 * the agent has a look of its own; always; never.
 */
@Composable
fun AgentGlyph(core: Core, type: String?, customName: String?, modifier: Modifier = Modifier, size: Dp = 16.dp, tab: TabRef? = null) {
    val c = LocalMc.current
    val mode by core.agentSymbol.collectAsState()
    val mixed by core.mixedAgents.collectAsState()
    val looks by core.looks.collectAsState()
    if (mode == 2) return
    // The tab's own look goes first, then the agent's (herdr name, then type).
    val tabLook = tab?.let { looks.tab(it.serverId, it.workspaceId, it.workspace, it.tab) } ?: Look()
    val agent = looks.agent(type, customName)
    val icon = tabLook.icon?.takeIf { Symbols.exists(it) } ?: agent.icon?.takeIf { Symbols.exists(it) }
    val own = icon != null || tabLook.ownColor() != null || agent.ownColor() != null
    if (mode == 0 && !own && !mixed) return
    if (icon == null && McIcons.agent(type) == null) return
    val tint = tabLook.ownColor() ?: agent.ownColor() ?: c.text
    Box(modifier) { SymbolMark(icon, tint, size, McIcons.agent(type), customName ?: type) }
}

/** Where a tab lives, to find its look. */
data class TabRef(val serverId: String, val workspaceId: String, val workspace: String, val tab: String?)

/** [AgentGlyph] for an agent row, with its tab's look. */
@Composable
fun AgentGlyph(core: Core, a: AgentRow, modifier: Modifier = Modifier, size: Dp = 16.dp) =
    AgentGlyph(core, a.agentType, a.customName, modifier, size, TabRef(a.serverId, a.workspaceId, a.workspace ?: "", a.tab))

/** Settings › Agents: symbol on/off, own colour and symbol per agent type or herdr name. */
@Composable
fun AgentsScreen(core: Core, nav: Nav) {
    val c = LocalMc.current
    val mode by core.agentSymbol.collectAsState()
    val state by core.state.collectAsState()
    val looks by core.looks.collectAsState()
    var open by remember { mutableStateOf<String?>(null) }
    // Types and herdr names seen now, plus every name that already has a look.
    val names = (state.agents.flatMap { listOfNotNull(it.agentType, it.customName) } + looks.agents.keys).distinct().sortedBy { it.lowercase() }
    val types = state.agents.mapNotNull { it.agentType }.toSet()

    Page(stringResource(R.string.agentlooks_agents_title), nav) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.agentlooks_agent_symbol))
            Block {
                Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Segmented(listOf(stringResource(R.string.agentlooks_symbol_auto), stringResource(R.string.agentlooks_symbol_always), stringResource(R.string.agentlooks_symbol_never)), mode.coerceIn(0, 2), { core.agentSymbol.set(it) }, Modifier.fillMaxWidth())
                    Text(
                        when (mode) {
                            0 -> stringResource(R.string.agentlooks_symbol_auto_hint)
                            1 -> stringResource(R.string.agentlooks_symbol_always_hint)
                            else -> stringResource(R.string.agentlooks_symbol_never_hint)
                        },
                        style = McType.caption, color = c.muted,
                    )
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.agentlooks_look_per_agent))
            Block {
                if (names.isEmpty()) Text(stringResource(R.string.agentlooks_no_agents), style = McType.small, color = c.muted, modifier = Modifier.padding(14.dp))
                names.forEachIndexed { i, name ->
                    if (i > 0) Divider()
                    val look = looks.agents[name] ?: Look()
                    val isType = name in types || name.lowercase() in setOf("claude", "codex")
                    val fallback = McIcons.agent(if (isType) name else null)
                    val tint = look.ownColor() ?: c.text
                    RowItem(name, if (isType) stringResource(R.string.agentlooks_agent_type) else stringResource(R.string.agentlooks_herdr_name), onClick = { open = if (open == name) null else name }) {
                        if (Symbols.exists(look.icon) || fallback != null) SymbolMark(look.icon, tint, 18.dp, fallback) else Text("–", style = McType.small, color = c.muted)
                        McIcon(if (open == name) McIcons.ChevronDown else McIcons.Chevron, c.muted, 16.dp)
                    }
                    if (open == name) LookEditor(look, isType) { core.setAgentLook(name, it) }
                }
            }
            Text(stringResource(R.string.agentlooks_agents_hint),
                style = McType.caption, color = c.muted, modifier = Modifier.padding(horizontal = 4.dp))
        }
    }
}

@Composable
private fun LookEditor(look: Look, isType: Boolean, onChange: (Look) -> Unit) {
    val c = LocalMc.current
    Column(Modifier.fillMaxWidth().background(c.inset).padding(14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        LookFields(look, ColorOwner.AGENT, if (isType) stringResource(R.string.agentlooks_default_icon) else stringResource(R.string.agentlooks_type_icon), onChange = onChange)
        SecondaryButton(stringResource(R.string.agentlooks_default), Modifier.fillMaxWidth()) { onChange(Look()) }
    }
}

@Composable
private fun LookCell(selected: Boolean, description: String, modifier: Modifier, onClick: () -> Unit, content: @Composable () -> Unit = { Text("A", style = McType.small, color = LocalMc.current.muted) }) {
    val c = LocalMc.current
    Box(
        modifier.height(40.dp).clip(RoundedCornerShape(10.dp)).background(if (selected) c.surface else Color.Transparent)
            .then(if (selected) Modifier.border(1.dp, c.text, RoundedCornerShape(10.dp)) else Modifier)
            .selectable(selected, role = Role.RadioButton, onClick = onClick).semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) { content() }
}
