package ch.inscribe.missionctrl.ui

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import ch.inscribe.missionctrl.Look
import ch.inscribe.missionctrl.WorkspacePrefs

/**
 * All looks and workspace preferences, held in memory and replaced as a whole on every change
 * (Core.looks). Rows read colours and symbols from here instead of the preferences file.
 *
 * Keys: workspaces by "name:<label>" or "id:<server>/<workspace>" (see [workspaceKey]); tabs by
 * "<workspace key>/<tab label>"; agents by type or herdr name.
 */
@Immutable
data class LookBook(
    val workspaces: Map<String, Look> = emptyMap(),
    val tabs: Map<String, Look> = emptyMap(),
    val agents: Map<String, Look> = emptyMap(),
    val wsPrefs: Map<String, WorkspacePrefs> = emptyMap(),
) {
    /** A workspace bound to its id has an "id:" entry; otherwise its name decides. */
    fun prefs(serverId: String, workspaceId: String, label: String): WorkspacePrefs =
        wsPrefs["id:$serverId/$workspaceId"] ?: wsPrefs["name:$label"] ?: WorkspacePrefs()

    fun workspaceKey(serverId: String, workspaceId: String, label: String): String =
        if (prefs(serverId, workspaceId, label).bindToName) "name:$label" else "id:$serverId/$workspaceId"

    fun workspace(serverId: String, workspaceId: String, label: String): Look = workspaces[workspaceKey(serverId, workspaceId, label)] ?: Look()

    fun tab(serverId: String, workspaceId: String, label: String, tab: String?): Look =
        if (tab.isNullOrEmpty()) Look() else tabs["${workspaceKey(serverId, workspaceId, label)}/$tab"] ?: Look()

    /** An agent's look: its herdr name first, then its type. */
    fun agent(type: String?, customName: String?): Look =
        customName?.let { agents[it] }?.takeIf { !it.isEmpty } ?: type?.let { agents[it] } ?: Look()

    /** Workspace colour: own, palette, automatic from the name; null for "No color". */
    fun workspaceColorOrNull(serverId: String, workspaceId: String, label: String): Color? {
        val look = workspace(serverId, workspaceId, label)
        look.rgb?.let { return rgbColor(it) }
        if (look.color == Look.NO_COLOR) return null
        return WorkspacePalette[(look.color ?: autoColorIndex(label)).coerceIn(0, WorkspacePalette.lastIndex)]
    }

    fun workspaceColor(serverId: String, workspaceId: String, label: String): Color =
        workspaceColorOrNull(serverId, workspaceId, label) ?: NO_COLOR_GREY

    /** Colour of a tab: its own, else its workspace's; null when neither has one. */
    fun paneColorOrNull(serverId: String, workspaceId: String, label: String, tab: String?): Color? =
        tab(serverId, workspaceId, label, tab).ownColor() ?: workspaceColorOrNull(serverId, workspaceId, label)

    fun workspaceSymbol(serverId: String, workspaceId: String, label: String): String? = workspace(serverId, workspaceId, label).icon

    // -- Changes (each returns a new book) ------------------------------------------------

    fun withWorkspace(serverId: String, workspaceId: String, label: String, look: Look) =
        copy(workspaces = workspaces.put(workspaceKey(serverId, workspaceId, label), look))

    fun withTab(serverId: String, workspaceId: String, label: String, tab: String, look: Look) =
        copy(tabs = tabs.put("${workspaceKey(serverId, workspaceId, label)}/$tab", look))

    fun withAgent(name: String, look: Look) = copy(agents = agents.put(name, look))

    /** A renamed tab keeps its look. */
    fun renamedTab(serverId: String, workspaceId: String, label: String, from: String, to: String): LookBook {
        if (from == to) return this
        val ws = workspaceKey(serverId, workspaceId, label)
        val look = tabs["$ws/$from"] ?: return this
        return copy(tabs = tabs - "$ws/$from" + ("$ws/$to" to look))
    }

    /**
     * New preferences for a workspace. Binding to the name stores them under the name (and
     * drops an id entry); binding to the id stores them under the id only, so other workspaces
     * with the same name keep theirs. Look and tab looks move along to the new key.
     */
    fun withPrefs(serverId: String, workspaceId: String, label: String, value: WorkspacePrefs): LookBook {
        val oldKey = workspaceKey(serverId, workspaceId, label)
        val idKey = "id:$serverId/$workspaceId"
        val nameKey = "name:$label"
        val prefs = if (value.bindToName) wsPrefs - idKey + (nameKey to value) else wsPrefs + (idKey to value)
        val newKey = if (value.bindToName) nameKey else idKey
        if (newKey == oldKey) return copy(wsPrefs = prefs)
        val moved = tabs.filterKeys { it.startsWith("$oldKey/") }.mapKeys { (k, _) -> newKey + k.removePrefix(oldKey) }
        return copy(
            wsPrefs = prefs,
            workspaces = workspaces.put(newKey, workspaces[oldKey] ?: Look()),
            tabs = tabs + moved,
        )
    }

    private fun Map<String, Look>.put(key: String, look: Look) = if (look.isEmpty) this - key else this + (key to look)

    companion object {
        /** Mark of a tab without any colour: a faint neutral grey, so rows stay aligned. */
        val NO_COLOR_GREY = Color(0x668A8A90)

        fun autoColorIndex(label: String) = Math.floorMod(label.hashCode(), WorkspacePalette.size)
    }
}

fun rgbColor(rgb: Int) = Color(0xFF000000.toInt() or (rgb and 0xFFFFFF))

/** The look's own colour: custom, else from the palette; null for automatic/none/standard. */
fun Look.ownColor(): Color? = rgb?.let(::rgbColor) ?: color?.let { WorkspacePalette.getOrNull(it) }
