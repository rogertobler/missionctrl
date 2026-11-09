package ch.inscribe.missionctrl

import android.content.Context
import android.util.Log
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import ch.inscribe.missionctrl.core.AgentRow
import ch.inscribe.missionctrl.core.Connection
import ch.inscribe.missionctrl.core.DirInfo
import ch.inscribe.missionctrl.core.FleetObserver
import ch.inscribe.missionctrl.core.FleetState
import ch.inscribe.missionctrl.core.KeyMaterial
import ch.inscribe.missionctrl.core.MissionCtrl
import ch.inscribe.missionctrl.core.PaneTerminal
import ch.inscribe.missionctrl.core.PaneTerminalObserver
import ch.inscribe.missionctrl.core.PaneText
import ch.inscribe.missionctrl.core.Status
import ch.inscribe.missionctrl.core.TabStart
import ch.inscribe.missionctrl.core.WorkspaceRow
import ch.inscribe.missionctrl.core.exportClientKey
import ch.inscribe.missionctrl.core.importClientKey
import ch.inscribe.missionctrl.core.initLogging
import ch.inscribe.missionctrl.ui.LiveTerminals
import ch.inscribe.missionctrl.ui.LookBook
import ch.inscribe.missionctrl.ui.Names
import ch.inscribe.missionctrl.ui.Nav
import ch.inscribe.missionctrl.ui.PaneTextTools
import ch.inscribe.missionctrl.ui.QuickKeys
import ch.inscribe.missionctrl.ui.Screen
import ch.inscribe.missionctrl.ui.TermKeys
import ch.inscribe.missionctrl.ui.ThemeMode
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** A persisted preference as a flow: [set] saves and publishes. */
class Pref<T> private constructor(private val flow: MutableStateFlow<T>, private val save: (T) -> Unit) : StateFlow<T> by flow {
    constructor(initial: T, save: (T) -> Unit) : this(MutableStateFlow(initial), save)

    fun set(value: T) { save(value); flow.value = value }
}

/**
 * Owns the Rust core and the app's persisted state, and exposes both as
 * flows for Compose. All core calls that block run on [Dispatchers.IO].
 * One per process ([CoreHolder]); it outlives activities, so the navigation and drafts live here.
 */
class Core(private val context: Context) : FleetObserver {
    val keys = KeyVault(context)
    val settings = Settings(context)
    val whisper = WhisperStore(context)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _state = MutableStateFlow(FleetState(emptyList(), emptyList(), emptyList()))
    /** Fleet as the app shows it: herdr's state plus [menuPanes] counted as waiting. */
    val state: StateFlow<FleetState> = _state.asStateFlow()
    @Volatile private var raw = FleetState(emptyList(), emptyList(), emptyList())
    private val _real = MutableStateFlow(FleetState(emptyList(), emptyList(), emptyList()))
    /** herdr's fleet as it is, also during the demo: the server settings show the real connections. */
    val real: StateFlow<FleetState> = _real.asStateFlow()

    /**
     * Panes whose agent herdr reports as resting although it shows a choice menu, "server|pane".
     * Claude's plan approval ("Would you like to proceed?") is such a case (herdr 0.9.5: idle).
     */
    @Volatile private var menuPanes = emptySet<String>()
    /** Until when a resting pane's screen is checked for a menu (set when it comes to rest). */
    private val menuCheckUntil = ConcurrentHashMap<String, Long>()

    // Per pane: last visible screen, its flow for the views, and what the detail keeps.
    private class Shot(val text: String, val at: Long)
    private val shots = ConcurrentHashMap<String, Shot>()
    private val screens = ConcurrentHashMap<String, StateFlow<String?>>()
    private val panes = ConcurrentHashMap<String, PaneView>()

    // Starts hidden: a process started by the service or a notification has no activity yet.
    private val foreground = MutableStateFlow(false)
    /** The app is on screen (resumed). Notifications are only posted while it is not. */
    val visible: Boolean get() = foreground.value
    /** Set by [FleetService] while it runs. */
    @Volatile var serviceRunning = false

    /** Where the user is; kept here so folding the phone (a new activity) keeps it. */
    val nav = Nav(Screen.Mission)
    /** Unsent text per "server|pane" (detail) or "card|server|pane" (waiting card). */
    val drafts = mutableStateMapOf<String, String>()

    // -- Preferences ---------------------------------------------------------------------

    val themeMode = Pref(ThemeMode.entries[settings.themeMode.coerceIn(0, 2)]) { settings.themeMode = it.ordinal }
    val accent = Pref(Color(settings.accent.toInt())) { settings.accent = it.toArgb().toLong() and 0xFFFFFFFFL }
    val groupByWorkspace = Pref(settings.groupByWorkspace) { settings.groupByWorkspace = it }
    val showDock = Pref(settings.showDock) { settings.showDock = it }
    val menuRight = Pref(settings.menuRight) { settings.menuRight = it }
    val hideKeyboardAfterSend = Pref(settings.hideKeyboardAfterSend) { settings.hideKeyboardAfterSend = it }
    /** Word suggestions and autocorrect in the answer field (learning stays off either way). */
    val keyboardSuggestions = Pref(settings.keyboardSuggestions) { settings.keyboardSuggestions = it }
    /** Button bars and the answer field 40 instead of 48 dp high (more rows for the terminal). */
    val compactKeys = Pref(settings.compactKeys) { settings.compactKeys = it }
    val sideCollapsed = Pref(settings.sideListCollapsed) { settings.sideListCollapsed = it }
    val sideWidth = Pref(settings.sideListWidth) { settings.sideListWidth = it }
    val agentSymbol = Pref(settings.agentSymbol) { settings.agentSymbol = it }
    val speech = Pref(settings.speech()) { settings.setSpeech(it) }
    val terminal = Pref(settings.terminal()) { settings.setTerminal(it) }
    /** Live terminals of the panes (TUI); they outlive their pages for a while. */
    private val liveTerminalsLazy = lazy { LiveTerminals(this, scope) }
    val liveTerminals by liveTerminalsLazy
    val pathPrefs = Pref(settings.pathPrefs()) { settings.setPathPrefs(it) }
    // Bars saved by older versions could hold up to 8 buttons.
    val startBar = Pref(settings.bar("start").take(QuickKeys.MAX_START)) { settings.setBar("start", it) }
    val detailBar = Pref(settings.bar("detail").take(QuickKeys.MAX_DETAIL)) { settings.setBar("detail", it) }
    fun bar(which: String) = if (which == "detail") detailBar else startBar

    // Background service and notifications (FleetService).
    val background = Pref(settings.background) { settings.background = it }
    val notifyWaiting = Pref(settings.notifyWaiting) { settings.notifyWaiting = it }
    val notifyDone = Pref(settings.notifyDone) { settings.notifyDone = it }
    val notifyConnection = Pref(settings.notifyConnection) { settings.notifyConnection = it }
    var backgroundAsked: Boolean
        get() = settings.backgroundAsked
        set(value) { settings.backgroundAsked = value }

    private val _pins = MutableStateFlow(settings.pins())
    val pins: StateFlow<List<Pin>> = _pins.asStateFlow()
    private val _keyList = MutableStateFlow(keys.list())
    val keyList: StateFlow<List<KeyVault.Entry>> = _keyList.asStateFlow()
    /**
     * Demo: on, the app shows [DemoFleet] instead of the servers (they stay connected
     * underneath). Declared before `init`: its first publish() reads it.
     */
    val demo = MutableStateFlow(false)
    private var demoBuilt: DemoFleet? = null
    /** The sample fleet, built anew when the demo is switched on or the app's language changed. */
    private val demoFleet: DemoFleet
        @Synchronized get() {
            val locale = context.resources.configuration.locales[0]
            demoBuilt?.takeIf { it.locale == locale }?.let { return it }
            demoBuilt?.stop()
            return DemoFleet(context, scope) { publish(); inputTick.update { it + 1 } }.also { demoBuilt = it }
        }

    /** Protected keys that wait for fingerprint, face or PIN; servers using them cannot connect yet. */
    private val _lockedKeys = MutableStateFlow<Set<String>>(emptySet())
    val lockedKeys: StateFlow<Set<String>> = _lockedKeys.asStateFlow()
    private val _servers = MutableStateFlow(settings.servers())
    val servers: StateFlow<List<SavedServer>> = _servers.asStateFlow()

    // -- Looks (in memory) ------------------------------------------------------

    private val _looks = MutableStateFlow(settings.looks())
    val looks: StateFlow<LookBook> = _looks.asStateFlow()

    private fun changeLooks(change: (LookBook) -> LookBook) {
        synchronized(_looks) {
            val old = _looks.value
            val next = change(old)
            if (next != old) { settings.saveLooks(old, next); _looks.value = next }
        }
    }

    fun setWorkspaceLook(ws: WorkspaceRow, look: Look) = changeLooks { it.withWorkspace(ws.serverId, ws.workspaceId, ws.label, look) }
    fun setTabLook(ws: WorkspaceRow, tab: String, look: Look) = changeLooks { it.withTab(ws.serverId, ws.workspaceId, ws.label, tab, look) }
    fun moveTabLook(ws: WorkspaceRow, from: String, to: String) = changeLooks { it.renamedTab(ws.serverId, ws.workspaceId, ws.label, from, to) }
    fun setAgentLook(name: String, look: Look) = changeLooks { it.withAgent(name, look) }
    fun setWorkspacePrefs(ws: WorkspaceRow, prefs: WorkspacePrefs) = changeLooks { it.withPrefs(ws.serverId, ws.workspaceId, ws.label, prefs) }

    /** Notifications for this agent: its server and its workspace allow them. */
    fun notifies(a: AgentRow): Boolean = !isDemo(a.serverId) &&
        serverNotifies(a.serverId) && looks.value.prefs(a.serverId, a.workspaceId, a.workspace ?: "").notify

    fun serverNotifies(serverId: String): Boolean = servers.value.firstOrNull { it.id == serverId }?.notify != false

    // -- Derived from the fleet ------------------------------------------------------------

    /** Tab labels used more than once (lists then add the workspace). */
    val duplicateTabs: StateFlow<Set<String>> =
        state.map { Names.duplicateTabs(it) }.distinctUntilChanged().stateIn(scope, SharingStarted.Eagerly, emptySet())

    /** Different agent types are running (the automatic agent symbol shows then). */
    val mixedAgents: StateFlow<Boolean> =
        state.map { s -> s.agents.mapNotNull { it.agentType?.lowercase() }.distinct().size > 1 }
            .distinctUntilChanged().stateIn(scope, SharingStarted.Eagerly, false)

    /** Git facts per "server|dir" and home directory per server, for the path line. */
    val dirs = MutableStateFlow(emptyMap<String, DirInfo>())
    val homes = MutableStateFlow(emptyMap<String, String>())

    /** A notification was tapped: the UI opens this pane ("server" to "pane") and clears it. */
    val openRequest = MutableStateFlow<Pair<String, String>?>(null)

    /**
     * Waiting cards, "server|pane": closed with ✕ ([Dismissal.question] null, until the agent stops
     * waiting) or answered (the question's signature: the card waits for the next one).
     */
    val dismissed = MutableStateFlow(emptyMap<String, Dismissal>())

    data class Dismissal(val question: String?, val at: Long = System.currentTimeMillis())

    /** When each pane's shown status last changed, "server|pane" → millis; only changes seen.
     * Declared before `init`: the first update arrives from there. */
    private val _statusSince = MutableStateFlow(emptyMap<String, Long>())
    val statusSince: StateFlow<Map<String, Long>> = _statusSince.asStateFlow()
    private var lastStatus = emptyMap<String, Status>()

    private val mc: MissionCtrl = run {
        initLogging()
        MissionCtrl(this)
    }

    init {
        _keyList.value.forEach { loadKey(it.id) }
        settings.trustedKeys().forEach { mc.trustHostKey(it.host, it.port.toUShort(), it.openssh) }
        _servers.value.filter { it.autoConnect }.forEach { connect(it) }
        onUpdate(mc.state())
        scope.launch { watchMenus() }
        scope.launch { watchDirs() }
    }

    // -- Fleet updates --------------------------------------------------------------------

    /** Called by the core on a background thread. */
    override fun onUpdate(state: FleetState) {
        val before = raw.agents.associate { key(it) to it.status }
        raw = state
        _real.value = state
        val until = System.currentTimeMillis() + MENU_CHECK_MS
        // A pane that just came to rest (or is new) may show a menu: check it for a while.
        for (a in state.agents) if (a.status.resting() && before[key(a)]?.resting() != true) menuCheckUntil[key(a)] = until
        publish()
        prune(state)
    }

    fun setDemo(on: Boolean) {
        // Each start begins with the question again, in the app's current language.
        if (on && !demo.value) synchronized(this) { demoBuilt?.stop(); demoBuilt = null }
        demo.value = on
        publish()
    }

    /** After a language change (the screens are recreated): the demo's sample texts follow it. */
    fun languageChanged() {
        if (demo.value) publish()
    }

    private fun isDemo(serverId: String) = serverId == DemoFleet.SERVER

    /** Actions that need a real server say so in the demo. */
    private fun notInDemo(serverId: String) {
        if (isDemo(serverId)) throw IllegalStateException(context.getString(R.string.demo_not_possible))
    }

    @Synchronized
    private fun publish() {
        val state = if (demo.value) demoFleet.state() else withMenus(raw, menuPanes)
        _state.value = state
        val now = System.currentTimeMillis()
        val statuses = state.agents.associate { key(it) to it.status }
        // A pane seen for the first time has no known start (the app may have just started).
        val changed = statuses.filter { (k, v) -> lastStatus[k] != null && lastStatus[k] != v }.keys
        if (changed.isNotEmpty() || _statusSince.value.keys.any { it !in statuses }) {
            _statusSince.value = _statusSince.value.filterKeys { it in statuses } + changed.associateWith { now }
        }
        lastStatus = statuses
        val waiting = state.agents.filter { it.status == Status.BLOCKED }.map(::key).toSet()
        dismissed.update { d -> d.filterKeys { it in waiting } }
    }

    /** Forgets what belonged to panes that are gone. */
    private fun prune(state: FleetState) {
        val alive = state.agents.map(::key).toSet() + state.workspaces.flatMap { ws -> ws.tabs.mapNotNull { t -> t.firstPaneId?.let { "${ws.serverId}|$it" } } }
        panes.keys.retainAll(alive); screens.keys.retainAll(alive); shots.keys.retainAll(alive); menuCheckUntil.keys.retainAll(alive)
        // Called off the main thread; the terminals live on it.
        if (liveTerminalsLazy.isInitialized()) scope.launch(Dispatchers.Main) { liveTerminals.retain(alive) }
    }

    /** Looks at the screens of resting agents for a choice menu; visible reads only (no scrolling). */
    private suspend fun watchMenus() {
        while (scope.isActive) {
            // On screen every 3 s; in the background only while the service runs, every 10 s.
            delay(if (visible) 3_000 else 10_000)
            if (!visible && !serviceRunning) continue
            val now = System.currentTimeMillis()
            val found = mutableSetOf<String>()
            for (a in raw.agents) {
                if (!a.status.resting()) continue
                val key = key(a)
                val known = key in menuPanes
                // Only right after coming to rest, and while a menu is up (to see it go).
                if (!known && (menuCheckUntil[key] ?: 0) < now) continue
                // While the app pages Claude's transcript, the screen is not the live one.
                if (panes[key]?.claude?.busy == true) { if (known) found += key; continue }
                val text = screenText(a.serverId, a.paneId, maxAgeMs = 2_500) ?: continue
                if (PaneTextTools.showsMenu(text)) found += key
            }
            if (found != menuPanes) { menuPanes = found; publish() }
        }
    }

    /**
     * Fetches branch and repository of the agents' directories over SSH, only while the path
     * line shows such a field and the app is on screen: new directories within seconds, known
     * ones again after a minute (branches change).
     */
    private suspend fun watchDirs() {
        val fetched = mutableMapOf<String, Long>()
        while (scope.isActive) {
            delay(4_000)
            if (!visible || !pathPrefs.value.needsDirs) continue
            val now = System.currentTimeMillis()
            for ((serverId, agents) in raw.agents.groupBy { it.serverId }) {
                if (raw.servers.firstOrNull { it.id == serverId }?.connection !is Connection.Online) continue
                val wanted = agents.mapNotNull { it.cwd }.distinct()
                if (wanted.isEmpty()) continue
                val known = wanted.all { "$serverId|$it" in dirs.value }
                if (known && now - (fetched[serverId] ?: 0) < 60_000) continue
                fetched[serverId] = now
                val report = runCatching { mc.dirInfo(serverId, wanted) }.onFailure { Log.w(TAG, "dir info $serverId: ${it.message}") }.getOrNull() ?: continue
                homes.update { it + (serverId to report.home) }
                dirs.update { old -> old + report.dirs.associateBy { "$serverId|${it.dir}" } }
            }
        }
    }

    // -- Screens, shared by cards, detail, menu check and notifications --------------------

    /** Visible screen of a pane (plain text), from the last read if it is fresh enough. */
    suspend fun screenText(serverId: String, paneId: String, maxAgeMs: Long = 1_000): String? {
        if (isDemo(serverId)) return demoFleet.screen(paneId).text
        val key = "$serverId|$paneId"
        shots[key]?.takeIf { System.currentTimeMillis() - it.at <= maxAgeMs }?.let { return it.text }
        val text = runCatching { mc.readScreen(serverId, paneId, false).text }.getOrNull() ?: return null
        shots[key] = Shot(text, System.currentTimeMillis())
        return text
    }

    /**
     * The visible screen as a flow: read every 3 s while someone watches and the app is on
     * screen, and right after every input. One read per pane, however many views watch it.
     */
    fun screen(serverId: String, paneId: String): StateFlow<String?> = screens.getOrPut("$serverId|$paneId") {
        flow {
            while (true) {
                awaitVisible()
                screenText(serverId, paneId)?.let { emit(it) }
                // The next input (let the agent redraw first) or 3 s, whichever comes first.
                if (withTimeoutOrNull(3_000) { inputTick.drop(1).first() } != null) delay(150)
            }
        }.stateIn(scope, SharingStarted.WhileSubscribed(2_000), shots["$serverId|$paneId"]?.text)
    }

    /** What the detail view keeps of a pane while the pane exists. */
    fun pane(serverId: String, paneId: String): PaneView = panes.getOrPut("$serverId|$paneId") { PaneView() }
    fun paneOrNull(serverId: String, paneId: String): PaneView? = panes["$serverId|$paneId"]

    suspend fun awaitVisible() { foreground.first { it } }
    /** The app is on screen, as a flow (the TUI gives its pane back when the app leaves). */
    val onScreen: StateFlow<Boolean> get() = foreground

    // -- Lifecycle ----------------------------------------------------------------------------

    fun shutdown() = mc.shutdown()

    private var releaseVoice: Job? = null

    /** App came to the foreground: reconnect what dropped, refresh the rest. */
    fun onForeground() {
        foreground.value = true
        releaseVoice?.cancel()
        mc.retryAll()
    }

    /** App left the screen: screens are only watched further if the background service runs. */
    fun onBackground() {
        foreground.value = false
        // A loaded Whisper model holds hundreds of MB; Android would end the process sooner.
        releaseVoice = scope.launch { delay(30_000); whisper.release() }
    }

    // -- Pins -------------------------------------------------------------------------------

    fun isPinned(serverId: String, tabId: String) = pins.value.any { it.serverId == serverId && it.tabId == tabId }

    fun togglePin(pin: Pin) {
        val next = if (isPinned(pin.serverId, pin.tabId)) pins.value.filterNot { it.serverId == pin.serverId && it.tabId == pin.tabId } else pins.value + pin
        settings.savePins(next); _pins.value = next
    }

    fun movePin(pin: Pin, delta: Int) {
        val list = pins.value.toMutableList()
        val i = list.indexOf(pin)
        val j = i + delta
        if (i < 0 || j !in list.indices) return
        list[i] = list[j]; list[j] = pin
        settings.savePins(list); _pins.value = list
    }

    fun dismiss(agent: AgentRow) = dismissed.update { it + (key(agent) to Dismissal(null)) }

    /** [question] was answered: its card shows "sent" until the next question (Claude asks several in a row). */
    fun answered(agent: AgentRow, question: String) = dismissed.update { it + (key(agent) to Dismissal(PaneTextTools.signature(question))) }

    // -- Servers ------------------------------------------------------------------------------

    suspend fun saveServer(server: SavedServer, password: String?) = io {
        if (password != null) keys.savePassword(server.id, password)
        val list = servers.value.toMutableList()
        val i = list.indexOfFirst { it.id == server.id }
        if (i >= 0) list[i] = server else list += server
        settings.saveServers(list)
        _servers.value = list
        connect(server)
    }

    fun removeServer(id: String) {
        settings.saveServers(servers.value.filterNot { it.id == id })
        _servers.value = settings.servers()
        keys.deletePassword(id)
        mc.removeServer(id)
    }

    /** Connect a passive server on demand. */
    fun connectNow(id: String) {
        servers.value.firstOrNull { it.id == id }?.let { connect(it) }
    }

    /** Reconnect one server now (notification action). */
    fun retry(serverId: String) = mc.retry(serverId)

    fun trust(serverId: String, unknown: Connection.HostKeyUnknown) {
        settings.addTrustedKey(Settings.TrustedKey(unknown.host, unknown.port.toInt(), unknown.openssh))
        mc.trustHostKey(unknown.host, unknown.port, unknown.openssh)
        mc.retry(serverId)
    }

    fun forgetHostKey(server: SavedServer) {
        settings.forgetTrustedKeys(server.host, server.port)
        // Drops live connections to that host as well.
        mc.forgetHostKeys(server.host, server.port.toUShort())
    }

    fun newServerId(): String = UUID.randomUUID().toString()

    private fun connect(server: SavedServer) {
        // A password that cannot be decrypted must not keep the app from starting.
        val password = runCatching { keys.password(server.id) }.onFailure { Log.w(TAG, "password ${server.id}: ${it.message}") }.getOrNull()
        mc.addServer(server.toConfig(password))
    }

    // -- Keys ---------------------------------------------------------------------------------

    /** [protect]: unlock with fingerprint, face or PIN just before (the new key is encrypted right away). */
    suspend fun generateKey(name: String, protect: Boolean): KeyVault.Entry =
        io { keys.generate(name, protect).also { loadKey(it.id); _keyList.value = keys.list() } }

    /** Parses without storing. Throws McException.PassphraseRequired / WrongPassphrase for the UI to handle. */
    suspend fun readKey(text: String, passphrase: String?): KeyMaterial = io { importClientKey(text, passphrase) }

    suspend fun addKey(name: String, material: KeyMaterial, protect: Boolean): KeyVault.Entry =
        io { keys.import(name, material, protect).also { loadKey(it.id); _keyList.value = keys.list() } }

    /** OpenSSH file encrypted with [passphrase]; a protected key needs an unlock first ([KeyVault.needsUnlock]). */
    suspend fun exportKey(id: String, passphrase: String): String = io { exportClientKey(keys.privateKey(id), passphrase) }

    fun deleteKey(id: String) {
        keys.delete(id)
        mc.removeKey(id)
        _lockedKeys.value -= id
        _keyList.value = keys.list()
    }

    fun renameKey(id: String, name: String) { keys.rename(id, name); _keyList.value = keys.list() }

    /** [serverId] cannot connect before its protected key is unlocked. */
    fun waitsForUnlock(serverId: String): Boolean =
        servers.value.firstOrNull { it.id == serverId }?.keyId?.let { it in _lockedKeys.value } == true

    /** True when a saved server waits for a locked key. */
    fun lockedInUse(): Boolean = _lockedKeys.value.let { locked -> locked.isNotEmpty() && _servers.value.any { it.keyId in locked } }

    /** Right after fingerprint, face or PIN: load the locked keys and let their servers connect. */
    suspend fun unlockKeys() = io {
        val loaded = _lockedKeys.value.filter { loadKey(it) }
        if (loaded.isNotEmpty()) mc.retryAll()
    }

    private fun loadKey(id: String): Boolean = runCatching { mc.addKey(id, keys.privateKey(id)) }
        .onSuccess { _lockedKeys.value -= id }
        .onFailure {
            if (KeyVault.needsUnlock(it)) _lockedKeys.value += id
            else Log.w(TAG, "key $id not loaded: ${it.message}")
        }
        .isSuccess

    // -- Pane and workspace actions (suspend: the core runs them on its own runtime) ----------------

    suspend fun readPane(serverId: String, paneId: String, lines: Int, ansi: Boolean = false): PaneText =
        if (isDemo(serverId)) demoFleet.screen(paneId) else mc.readPane(serverId, paneId, lines.toUInt(), ansi)

    /** Visible screen only; safe to poll (herdr never scrolls the agent for it). */
    suspend fun readScreen(serverId: String, paneId: String, ansi: Boolean = false): PaneText =
        if (isDemo(serverId)) demoFleet.screen(paneId) else mc.readScreen(serverId, paneId, ansi)

    suspend fun dirInfo(serverId: String, dirs: List<String>) = notInDemo(serverId).let { mc.dirInfo(serverId, dirs) }

    /** Live terminal of a pane at [cols] × [rows] (TUI mode); release it to give the size back. */
    suspend fun openTerminal(serverId: String, paneId: String, cols: Int, rows: Int, observer: PaneTerminalObserver): PaneTerminal {
        if (isDemo(serverId)) throw IllegalStateException(context.getString(R.string.demo_no_terminal))
        return mc.openTerminal(serverId, paneId, cols.toUShort(), rows.toUShort(), observer)
    }

    // Input goes out one after the other: every send is its own herdr request, and quick
    // taps (↓ ↓ ⏎) must not overtake each other. The lock is fair, so taps keep their order.
    private val inputLock = Mutex()

    /** Bumped after every input, so views read the screen again right away. */
    val inputTick = MutableStateFlow(0L)

    suspend fun sendKeys(serverId: String, paneId: String, keys: List<String>) {
        if (isDemo(serverId)) return demoFleet.input(paneId, keys = keys)
        inputLock.withLock { mc.sendKeys(serverId, paneId, keys) }.also { inputTick.update { it + 1 } }
    }

    suspend fun sendText(serverId: String, paneId: String, text: String, enter: Boolean) {
        if (isDemo(serverId)) return demoFleet.input(paneId, text = text.takeIf { enter })
        inputLock.withLock { mc.sendText(serverId, paneId, TermKeys.message(text), enter) }.also { inputTick.update { it + 1 } }
    }

    /**
     * Text as the answer to a waiting agent, routed by the screen as it is now
     * ([PaneTextTools.route]): into a menu's free-text row ("Type something."), or after picking
     * "Chat about this" as a chat message ([PaneTextTools.TextRoute.CHAT]: the caller opens the terminal),
     * into Codex's notes field or after its "No, and tell Codex …" as a new message.
     */
    suspend fun answerText(serverId: String, paneId: String, text: String): PaneTextTools.TextRoute {
        if (isDemo(serverId)) { demoFleet.input(paneId, text = text); return PaneTextTools.TextRoute.PLAIN }
        val (route, before) = runCatching { PaneTextTools.route(readScreen(serverId, paneId).text) }
            .getOrDefault(PaneTextTools.TextRoute.PLAIN to emptyList())
        inputLock.withLock {
            if (before.isNotEmpty()) {
                mc.sendKeys(serverId, paneId, before)
                // The agent redraws first: the row (or Codex's notes field) becomes an input, or the
                // question closes for a message.
                val closes = route == PaneTextTools.TextRoute.CHAT || route == PaneTextTools.TextRoute.MESSAGE
                delay(if (closes) 700 else 250)
            }
            mc.sendText(serverId, paneId, TermKeys.message(text), true)
        }
        inputTick.update { it + 1 }
        return route
    }

    suspend fun upload(serverId: String, name: String, data: ByteArray): String = notInDemo(serverId).let { mc.uploadFile(serverId, name, data) }

    /** The plan Claude asks about ([PaneTextTools.planPath]); the core reads only plan files. */
    suspend fun readPlan(serverId: String, path: String): String = notInDemo(serverId).let { mc.readPlan(serverId, path) }

    suspend fun createTab(serverId: String, workspaceId: String, label: String?, cwd: String?, start: TabStart): String =
        notInDemo(serverId).let { _ -> mc.createTab(serverId, workspaceId, label, cwd, start) }

    suspend fun createWorkspace(serverId: String, label: String?, cwd: String?, start: TabStart): String =
        notInDemo(serverId).let { _ -> mc.createWorkspace(serverId, label, cwd, start) }

    suspend fun closeTab(serverId: String, tabId: String) = notInDemo(serverId).let { mc.closeTab(serverId, tabId) }

    /** Where a tab stands in its workspace and how many there are; null if unknown. */
    fun tabPosition(serverId: String, tabId: String): Pair<Int, Int>? {
        val ws = state.value.workspaces.firstOrNull { w -> w.serverId == serverId && w.tabs.any { it.tabId == tabId } } ?: return null
        return ws.tabs.indexOfFirst { it.tabId == tabId } to ws.tabs.size
    }

    /** Where a workspace stands among its server's workspaces, and how many there are. */
    fun workspacePosition(serverId: String, workspaceId: String): Pair<Int, Int>? {
        val list = state.value.workspaces.filter { it.serverId == serverId }
        val i = list.indexOfFirst { it.workspaceId == workspaceId }
        return if (i < 0) null else i to list.size
    }

    /**
     * Moves a tab one place up (−1) or down (+1) in its workspace, on the server, so every view
     * and the laptop show the new order. herdr inserts before the entry at `insert_index`
     * of the list as it is before the move.
     */
    suspend fun moveTab(serverId: String, tabId: String, delta: Int) {
        notInDemo(serverId)
        val (i, n) = tabPosition(serverId, tabId) ?: return
        if (i + delta !in 0 until n) return
        mc.moveTab(serverId, tabId, insertIndex(i, delta))
    }

    /** Moves a workspace one place up (−1) or down (+1) among its server's workspaces. */
    suspend fun moveWorkspace(serverId: String, workspaceId: String, delta: Int) {
        notInDemo(serverId)
        val (i, n) = workspacePosition(serverId, workspaceId) ?: return
        if (i + delta !in 0 until n) return
        mc.moveWorkspace(serverId, workspaceId, insertIndex(i, delta))
    }

    private fun insertIndex(from: Int, delta: Int): UInt = (if (delta < 0) from - 1 else from + 2).coerceAtLeast(0).toUInt()

    suspend fun closeWorkspace(serverId: String, workspaceId: String) = notInDemo(serverId).let { mc.closeWorkspace(serverId, workspaceId) }

    suspend fun renameWorkspace(serverId: String, workspaceId: String, label: String) = notInDemo(serverId).let { mc.renameWorkspace(serverId, workspaceId, label) }

    suspend fun renameTab(serverId: String, tabId: String, label: String) = notInDemo(serverId).let { mc.renameTab(serverId, tabId, label) }

    private suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }

    companion object {
        private const val TAG = "MissionCtrl"
        /** How long after coming to rest a pane is checked for a choice menu. */
        private const val MENU_CHECK_MS = 90_000L

        fun key(a: AgentRow) = "${a.serverId}|${a.paneId}"
    }
}

fun Status.resting() = this == Status.IDLE || this == Status.DONE

/** Agents in [menus] count as waiting, and so do their tabs and workspaces. */
fun withMenus(state: FleetState, menus: Set<String>): FleetState {
    if (menus.isEmpty()) return state
    val agents = state.agents.map { if (Core.key(it) in menus && it.status.resting()) it.copy(status = Status.BLOCKED) else it }
    val tabs = agents.filter { it.status == Status.BLOCKED }.map { it.serverId to it.tabId }.toSet()
    val workspaces = state.workspaces.map { ws ->
        val wsTabs = ws.tabs.map { if ((ws.serverId to it.tabId) in tabs) it.copy(status = Status.BLOCKED) else it }
        ws.copy(tabs = wsTabs, status = if (wsTabs.any { it.status == Status.BLOCKED }) Status.BLOCKED else ws.status)
    }
    return state.copy(agents = agents, workspaces = workspaces)
}
