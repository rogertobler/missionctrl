package ch.inscribe.missionctrl.ui

import android.app.Activity
import android.content.ContextWrapper
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import ch.inscribe.missionctrl.AppLanguage
import ch.inscribe.missionctrl.R

/** Settings › Language: System, then every language the app has texts for, each in its own name. */
@Composable
fun LanguageScreen(nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    val activity = remember(context) { generateSequence(context) { (it as? ContextWrapper)?.baseContext }.filterIsInstance<Activity>().firstOrNull() }
    var current by remember { mutableStateOf(AppLanguage.current(context)) }
    fun pick(tag: String) {
        if (tag == current || activity == null) return
        current = tag
        AppLanguage.set(activity, tag)
    }
    Page(stringResource(R.string.language), nav) {
        Block {
            ChoiceRow(stringResource(R.string.language_system), stringResource(R.string.language_system_hint), current.isEmpty()) { pick("") }
            AppLanguage.available.forEach { tag ->
                Divider()
                ChoiceRow(AppLanguage.displayName(tag), null, current == tag) { pick(tag) }
            }
        }
    }
}
