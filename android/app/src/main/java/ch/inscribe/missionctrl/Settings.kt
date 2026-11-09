package ch.inscribe.missionctrl

import android.content.Context
import android.content.SharedPreferences
import androidx.core.content.edit
import ch.inscribe.missionctrl.core.ServerAuth
import ch.inscribe.missionctrl.core.ServerConfig
import ch.inscribe.missionctrl.ui.LookBook
import ch.inscribe.missionctrl.ui.TerminalPrefs
import ch.inscribe.missionctrl.ui.TerminalScheme
import ch.inscribe.missionctrl.ui.TerminalSchemes
import kotlin.properties.ReadWriteProperty
import kotlin.reflect.KProperty
import org.json.JSONArray
import org.json.JSONObject

/** Look of a workspace, tab or agent: colour (palette index or own) and symbol, all optional. */
data class Look(
    val color: Int? = null,
    val icon: String? = null,
    /** Own colour as 0xRRGGBB; goes before [color] (palette index). */
    val rgb: Int? = null,
) {
    val isEmpty: Boolean get() = color == null && icon == null && rgb == null

    fun toJson(): String = JSONObject().apply {
        color?.let { put("color", it) }
        icon?.let { put("icon", it) }
        rgb?.let { put("rgb", it) }
    }.toString()

    companion object {
        /** Workspace colour "No color" (null means automatic for workspaces). */
        const val NO_COLOR = -1

        fun fromJson(raw: String): Look {
            val j = JSONObject(raw)
            return Look(
                color = if (j.has("color")) j.getInt("color") else null,
                icon = j.optString("icon").ifEmpty { null },
                rgb = if (j.has("rgb")) j.getInt("rgb") else null,
            )
        }
    }
}

/** Per-workspace preferences. */
data class WorkspacePrefs(val notify: Boolean = true, val bindToName: Boolean = true) {
    fun toJson(): String = JSONObject().put("notify", notify).put("bind", bindToName).toString()

    companion object {
        fun fromJson(raw: String): WorkspacePrefs = JSONObject(raw).let {
            WorkspacePrefs(it.optBoolean("notify", true), it.optBoolean("bind", true))
        }
    }
}

/** A tab pinned to the dock. */
data class Pin(val serverId: String, val tabId: String, val workspaceLabel: String, val tabLabel: String)

/** Server as stored by the app. The password itself lives in [KeyVault]. */
data class SavedServer(
    val id: String,
    val name: String,
    val host: String,
    val port: Int,
    val user: String,
    val session: String?,
    /** Key id, or null for password login. */
    val keyId: String?,
    val autoConnect: Boolean = true,
    /** Notifications for this server's agents and its connection. */
    val notify: Boolean = true,
)

/**
 * App settings: servers, trusted host keys, looks and view preferences.
 * App-private and excluded from every backup.
 */
class Settings(context: Context) {
    data class TrustedKey(val host: String, val port: Int, val openssh: String)

    private val prefs = context.getSharedPreferences("missionctrl", Context.MODE_PRIVATE)

    // -- Simple values ----------------------------------------------------------------

    var groupByWorkspace by bool("group_by_workspace", false)
    var showDock by bool("show_dock", true)
    /** ☰ on the right (next to ⋯, which is always there) instead of on the left. */
    var menuRight by bool("menu_right", false)
    /** After sending typed text: close the keyboard (true) or keep it open for the next message. */
    var hideKeyboardAfterSend by bool("hide_keyboard_after_send", true)
    var keyboardSuggestions by bool("keyboard_suggestions", false)
    var compactKeys by bool("compact_keys", false)
    /** 0 = system, 1 = light, 2 = dark */
    var themeMode by int("theme_mode", 0)
    var accent by long("accent", 0xFF00E28A)
    var sideListCollapsed by bool("side_collapsed", false)
    /** Width of the side list on wide screens, in dp. */
    var sideListWidth by float("side_width", 300f)
    /** 0 = only when different agent types are around, 1 = always, 2 = never. */
    var agentSymbol by int("agent_symbol", 0)

    // Background
    /** Keep the connections in a foreground service while the app is not on screen. */
    var background by bool("background", false)
    /** The one-time question on first start (stay connected in the background?) was answered. */
    var backgroundAsked by bool("background_asked", false)
    var notifyWaiting by bool("notify_waiting", true)
    var notifyDone by bool("notify_done", true)
    var notifyConnection by bool("notify_connection", true)

    // -- Servers ------------------------------------------------------------------------

    fun servers(): List<SavedServer> = objects("servers").map {
        SavedServer(
            id = it.getString("id"),
            name = it.getString("name"),
            host = it.getString("host"),
            port = it.getInt("port"),
            user = it.getString("user"),
            session = it.optString("session").ifEmpty { null },
            // Servers from the first build used the single device key.
            keyId = if (it.has("key_id")) it.optString("key_id").ifEmpty { null } else KeyVault.DEVICE_KEY_ID,
            autoConnect = it.optBoolean("auto_connect", true),
            notify = it.optBoolean("notify", true),
        )
    }

    fun saveServers(servers: List<SavedServer>) = putObjects(
        "servers",
        servers.map {
            JSONObject()
                .put("id", it.id).put("name", it.name).put("host", it.host)
                .put("port", it.port).put("user", it.user).put("session", it.session ?: "")
                .put("key_id", it.keyId ?: "").put("auto_connect", it.autoConnect).put("notify", it.notify)
        },
    )

    // -- Host keys ----------------------------------------------------------------------

    fun trustedKeys(): List<TrustedKey> = objects("trusted_keys").map {
        TrustedKey(it.getString("host"), it.getInt("port"), it.getString("openssh"))
    }

    fun addTrustedKey(key: TrustedKey) = putTrusted((trustedKeys() + key).distinct())

    fun forgetTrustedKeys(host: String, port: Int) =
        putTrusted(trustedKeys().filterNot { it.host.equals(host, ignoreCase = true) && it.port == port })

    private fun putTrusted(keys: List<TrustedKey>) = putObjects(
        "trusted_keys",
        keys.map { JSONObject().put("host", it.host).put("port", it.port).put("openssh", it.openssh) },
    )

    // -- Looks and workspace preferences ------------------------------------------------

    /**
     * Everything about looks, read once: `look_ws_<ws-key>`, `look_tab_<ws-key>/<tab>`,
     * `look_agent_<name>` and `ws_prefs_<name:…|id:…>`. [LookBook] answers lookups from memory.
     */
    fun looks(): LookBook {
        val ws = mutableMapOf<String, Look>()
        val tabs = mutableMapOf<String, Look>()
        val agents = mutableMapOf<String, Look>()
        val wsPrefs = mutableMapOf<String, WorkspacePrefs>()
        for ((key, value) in prefs.all) {
            val raw = value as? String ?: continue
            runCatching {
                when {
                    key.startsWith(LOOK_WS) -> ws[key.removePrefix(LOOK_WS)] = Look.fromJson(raw)
                    key.startsWith(LOOK_TAB) -> tabs[key.removePrefix(LOOK_TAB)] = Look.fromJson(raw)
                    key.startsWith(LOOK_AGENT) -> agents[key.removePrefix(LOOK_AGENT)] = Look.fromJson(raw)
                    key.startsWith(WS_PREFS) -> wsPrefs[key.removePrefix(WS_PREFS)] = WorkspacePrefs.fromJson(raw)
                }
            }
        }
        return LookBook(ws, tabs, agents, wsPrefs)
    }

    /** Writes the entries of [next] that differ from [old] (an empty look removes its key). */
    fun saveLooks(old: LookBook, next: LookBook) {
        prefs.edit {
            fun <T> diff(prefix: String, a: Map<String, T>, b: Map<String, T>, json: (T) -> String) {
                (a.keys - b.keys).forEach { remove(prefix + it) }
                b.forEach { (k, v) -> if (a[k] != v) putString(prefix + k, json(v)) }
            }
            diff(LOOK_WS, old.workspaces, next.workspaces) { it.toJson() }
            diff(LOOK_TAB, old.tabs, next.tabs) { it.toJson() }
            diff(LOOK_AGENT, old.agents, next.agents) { it.toJson() }
            diff(WS_PREFS, old.wsPrefs, next.wsPrefs) { it.toJson() }
        }
    }

    // -- Pins ---------------------------------------------------------------------------

    fun pins(): List<Pin> = objects("pins").map {
        Pin(it.getString("server"), it.getString("tab"), it.getString("ws"), it.getString("label"))
    }

    fun savePins(pins: List<Pin>) = putObjects(
        "pins",
        pins.map { JSONObject().put("server", it.serverId).put("tab", it.tabId).put("ws", it.workspaceLabel).put("label", it.tabLabel) },
    )

    // -- Button bars --------------------------------------------------------------------

    fun bar(which: String): List<String> = prefs.getString("bar_$which", null)?.let { JSONArray(it).strings() } ?: defaultBar(which)

    fun setBar(which: String, keys: List<String>) = prefs.edit { putString("bar_$which", JSONArray(keys).toString()) }

    fun defaultBar(which: String) = when (which) {
        "detail" -> listOf("mic", "kb", "down", "up", "file", "enter")
        else -> listOf("mic", "kb", "down", "up", "enter")
    }

    // -- Speech, terminal, path line --------------------------------------------------------

    fun speech(): SpeechPrefs {
        val j = prefs.getString("speech", null)?.let(::JSONObject) ?: return SpeechPrefs()
        val repl = j.optJSONArray("replacements") ?: JSONArray()
        return SpeechPrefs(
            engine = j.optString("engine", "device").let { e ->
                // A service we no longer offer: back to the own model if one was chosen, else the device.
                if (SpeechPrefs.unusable(e)) (if (j.optString("whisper_model").isNotEmpty()) "whisper" else "device") else e
            },
            language = j.optString("language", "de-CH"),
            vocabulary = j.optJSONArray("vocabulary")?.strings().orEmpty(),
            replacements = (0 until repl.length()).map { repl.getJSONArray(it).let { a -> a.getString(0) to a.getString(1) } },
            autoSend = j.optBoolean("auto_send", false),
            whisperModel = j.optString("whisper_model", ""),
            whisperLive = j.optBoolean("whisper_live", true),
            stopAfterMs = j.optInt("stop_after_ms", SpeechPrefs.STOP_AFTER_MS),
        )
    }

    fun setSpeech(s: SpeechPrefs) {
        val j = JSONObject()
            .put("engine", s.engine).put("language", s.language)
            .put("vocabulary", JSONArray(s.vocabulary))
            .put("replacements", JSONArray(s.replacements.map { JSONArray(listOf(it.first, it.second)) }))
            .put("auto_send", s.autoSend).put("whisper_model", s.whisperModel).put("whisper_live", s.whisperLive).put("stop_after_ms", s.stopAfterMs)
        prefs.edit { putString("speech", j.toString()) }
    }

    fun terminal(): TerminalPrefs {
        val j = prefs.getString("terminal", null)?.let(::JSONObject) ?: return TerminalPrefs()
        val custom = j.optJSONArray("custom") ?: JSONArray()
        return TerminalPrefs(
            schemeId = j.optString("scheme", TerminalSchemes.AUTO),
            custom = (0 until custom.length()).map { TerminalScheme.fromJson(custom.getJSONObject(it)) },
            fontSize = j.optDouble("font", 12.5).toFloat(),
            tui = j.optBoolean("tui", false),
            lingerSeconds = j.optInt("linger", 20),
        )
    }

    fun setTerminal(t: TerminalPrefs) {
        val j = JSONObject().put("scheme", t.schemeId).put("font", t.fontSize.toDouble()).put("tui", t.tui).put("linger", t.lingerSeconds)
            .put("custom", JSONArray(t.custom.map { it.toJson() }))
        prefs.edit { putString("terminal", j.toString()) }
    }

    fun pathPrefs(): PathPrefs {
        val j = prefs.getString("path_line", null)?.let(::JSONObject) ?: return PathPrefs()
        val order = j.optJSONArray("order")?.strings() ?: PathPrefs.FIELDS
        val on = j.optJSONArray("on")?.strings()?.toSet() ?: PathPrefs().on
        // Fields added in later versions go to the end, switched off.
        return PathPrefs(order.filter { it in PathPrefs.FIELDS } + PathPrefs.FIELDS.filter { it !in order }, on, j.optString("sep", "/"))
    }

    fun setPathPrefs(p: PathPrefs) = prefs.edit {
        putString("path_line", JSONObject().put("order", JSONArray(p.order)).put("on", JSONArray(p.on.toList())).put("sep", p.sep).toString())
    }

    // -- Helpers ----------------------------------------------------------------------------

    private fun objects(name: String): List<JSONObject> {
        val json = JSONArray(prefs.getString(name, "[]"))
        return (0 until json.length()).map { json.getJSONObject(it) }
    }

    private fun putObjects(name: String, items: List<JSONObject>) {
        prefs.edit { putString(name, JSONArray(items).toString()) }
    }

    private fun <T> pref(key: String, default: T, get: SharedPreferences.(String, T) -> T, put: SharedPreferences.Editor.(String, T) -> Unit) =
        object : ReadWriteProperty<Any?, T> {
            override fun getValue(thisRef: Any?, property: KProperty<*>): T = prefs.get(key, default)
            override fun setValue(thisRef: Any?, property: KProperty<*>, value: T) = prefs.edit { put(key, value) }
        }

    private fun bool(key: String, default: Boolean) = pref(key, default, SharedPreferences::getBoolean, SharedPreferences.Editor::putBoolean)
    private fun int(key: String, default: Int) = pref(key, default, SharedPreferences::getInt, SharedPreferences.Editor::putInt)
    private fun long(key: String, default: Long) = pref(key, default, SharedPreferences::getLong, SharedPreferences.Editor::putLong)
    private fun float(key: String, default: Float) = pref(key, default, SharedPreferences::getFloat, SharedPreferences.Editor::putFloat)

    private companion object {
        const val LOOK_WS = "look_ws_"
        const val LOOK_TAB = "look_tab_"
        const val LOOK_AGENT = "look_agent_"
        const val WS_PREFS = "ws_prefs_"
    }
}

fun JSONArray.strings(): List<String> = (0 until length()).map { getString(it) }

// -- Path line ----------------------------------------------------------------

/** What the monospace line under an agent's name shows, in which order. */
data class PathPrefs(
    val order: List<String> = FIELDS,
    // Lists name the agent by its tab, so the line shows where it lives.
    val on: Set<String> = setOf("server", "workspace", "branch"),
    /** Separator; "/" without spaces, the others with a space on each side. */
    val sep: String = "/",
) {
    val needsDirs: Boolean get() = on.any { it == "branch" || it == "repo" || it == "path" }

    companion object {
        val FIELDS = listOf("server", "workspace", "tab", "branch", "dir", "path", "repo", "agent", "title")
        val SEPARATORS = listOf("/", "·", "›", ":", "|")
        // Field names in the app's language: pathFieldLabel in ui/AgentLooks.kt.
    }
}

// -- Speech -------------------------------------------------------------------

/** Speech settings, modelled on voxtype (prompt words, replacements); Swiss German writes ss for ß. */
data class SpeechPrefs(
    /** "device" = on-device recognizer, "whisper" = own model, or "service:<package>/<class>". */
    val engine: String = "device",
    /** BCP-47 tag, e.g. "de-CH"; empty = system language. */
    val language: String = "de-CH",
    /** Words the recognizer should expect (like voxtype's initial_prompt). */
    val vocabulary: List<String> = emptyList(),
    /** Applied after recognition, in order (like voxtype's [text] replacements). */
    val replacements: List<Pair<String, String>> = emptyList(),
    /** Send right after speaking instead of placing the text in the input. */
    val autoSend: Boolean = false,
    /** Whisper model id (see [WhisperStore.MODELS]) for engine "whisper". */
    val whisperModel: String = "",
    /** Whisper recognizes sentence by sentence at short pauses, so text shows while speaking. */
    val whisperLive: Boolean = true,
    /** Pause that ends a take, in ms; 0 = only with ✓. */
    val stopAfterMs: Int = STOP_AFTER_MS,
) {
    /** Swiss German has no ß (also when the system language is Swiss German). */
    val swissSpelling: Boolean
        get() = language.ifEmpty { java.util.Locale.getDefault().toLanguageTag() }.equals("de-CH", ignoreCase = true)

    companion object {
        const val STOP_AFTER_MS = 3_500
        /** Choices in the settings: after a pause of 2, 3.5 or 6 s, or only with ✓. */
        val STOP_CHOICES = listOf(2_000, 3_500, 6_000, 0)

        /**
         * Recognition services that refuse other apps. Claude's checks the microphone permission
         * with its own uid and our package name and always ends in ERROR_CLIENT (seen on a Galaxy Z Fold8).
         */
        private val UNUSABLE_PACKAGES = setOf("com.anthropic.claude")

        fun unusable(engine: String): Boolean =
            engine.startsWith("service:") && engine.removePrefix("service:").substringBefore('/') in UNUSABLE_PACKAGES
    }
}

/** Convert a saved server to the core's config, given its password if any. */
fun SavedServer.toConfig(password: String?): ServerConfig = ServerConfig(
    id = id,
    name = name,
    host = host,
    port = port.toUShort(),
    user = user,
    session = session,
    auth = if (keyId != null) ServerAuth.Key(keyId) else ServerAuth.Password(password ?: ""),
)
