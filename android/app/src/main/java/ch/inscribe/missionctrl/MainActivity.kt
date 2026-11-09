package ch.inscribe.missionctrl

import android.app.Application
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import ch.inscribe.missionctrl.ui.McApp
import ch.inscribe.missionctrl.ui.ThemeMode

/** Holds the one [Core] of the process, so recreated activities keep their connections. */
class McApplication : Application() {
    val core: Core by lazy { Core(this) }

    override fun onCreate() {
        super.onCreate()
        AppLanguage.migrate(this)
    }

    override fun attachBaseContext(base: Context) = super.attachBaseContext(AppLanguage.wrap(base))
}

object CoreHolder {
    fun get(context: Context): Core = (context.applicationContext as McApplication).core
}

class MainActivity : ComponentActivity() {
    private val core: Core get() = CoreHolder.get(this)

    override fun attachBaseContext(newBase: Context) = super.attachBaseContext(AppLanguage.wrap(newBase))

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Channel names in the app's language: Android shows them in its notification settings (also after a language change).
        Notifications.channels(this)
        core.languageChanged()
        setContent {
            val mode by core.themeMode.collectAsState()
            val dark = when (mode) { ThemeMode.SYSTEM -> isSystemInDarkTheme(); ThemeMode.DARK -> true; ThemeMode.LIGHT -> false }
            // Status bar icons follow the app's mode, not the system's.
            DisposableEffect(dark) {
                val style = if (dark) SystemBarStyle.dark(Color.TRANSPARENT) else SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
                enableEdgeToEdge(statusBarStyle = style, navigationBarStyle = style)
                onDispose {}
            }
            McApp(core)
        }
        // Only a fresh start: a recreated activity (folding, rotating) already handled the intent.
        if (savedInstanceState == null) openFrom(intent)
    }

    /** A tapped notification brings the app back with the pane to open. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        openFrom(intent)
    }

    private fun openFrom(intent: Intent?) {
        val server = intent?.getStringExtra(FleetService.EXTRA_SERVER) ?: return
        val pane = intent.getStringExtra(FleetService.EXTRA_PANE) ?: return
        core.openRequest.value = server to pane
    }

    override fun onStart() {
        super.onStart()
        // Started while visible, as Android requires; it then keeps running in the background.
        if (core.background.value) FleetService.start(this)
    }

    override fun onResume() {
        super.onResume()
        core.onForeground()
        Notifications.clearAgents(this)
    }

    override fun onPause() {
        core.onBackground()
        super.onPause()
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= TRIM_MEMORY_BACKGROUND) core.whisper.release()
    }
}
