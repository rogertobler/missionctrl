package ch.inscribe.missionctrl.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import ch.inscribe.missionctrl.R
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Licence texts come hard-wrapped at about 80 characters; on a phone that breaks every line twice.
 * [reflow] joins the lines of a paragraph again so the text wraps to the screen. Kept apart: short
 * lines (titles, addresses), list items, copyright lines, comment prefixes and rules (---).
 */
internal object LicenseText {
    private const val MIN_WRAPPED = 50
    private val keepApart = Regex("""^\s*([-*•+o]|\(?[0-9a-zA-Z]{1,3}[.)]|\[[0-9]+])\s|^\s*(Copyright\b|\([cC]\)|©|//|#|\*)""")
    private val rule = Regex("""^\s*[-=_*#~]{3,}\s*$""")
    private val hyphenEnd = Regex("""[a-z]-$""")

    fun reflow(text: String): String {
        val out = mutableListOf<String>()
        for (line in text.replace("\r\n", "\n").split('\n')) {
            val prev = out.lastOrNull()?.trimEnd()
            val next = line.trim()
            if (prev != null && prev.isNotBlank() && next.isNotEmpty() && prev.length >= MIN_WRAPPED &&
                !keepApart.containsMatchIn(line) && !rule.matches(line) && !rule.matches(prev)
            ) {
                out[out.lastIndex] = if (hyphenEnd.containsMatchIn(prev) && next.first().isLetter()) prev + next else "$prev $next"
            } else {
                out += line.trimEnd()
            }
        }
        return out.joinToString("\n")
    }
}

/** One library, font or model and the keys of its licence texts. */
private data class Licensed(val name: String, val version: String, val license: String, val url: String, val texts: List<String>)

private class ThirdParty(val groups: List<Pair<String, List<Licensed>>>, val texts: Map<String, String>)

/** assets/licenses/third_party.json, written by android/scripts/third_party_licenses.py. */
private fun readThirdParty(context: Context): ThirdParty {
    val json = JSONObject(context.assets.open("licenses/third_party.json").bufferedReader().use { it.readText() })
    val texts = json.getJSONObject("texts").let { t -> t.keys().asSequence().associateWith { t.getString(it) } }
    val groups = json.getJSONArray("groups").let { arr ->
        (0 until arr.length()).map { arr.getJSONObject(it) }.map { g ->
            val items = g.getJSONArray("items").let { a ->
                (0 until a.length()).map { a.getJSONObject(it) }.map { i ->
                    val ids = i.getJSONArray("texts").let { t -> (0 until t.length()).map { t.getString(it) } }
                    Licensed(i.getString("name"), i.optString("version"), i.getString("license"), i.optString("url"), ids)
                }
            }
            g.getString("key") to items
        }
    }
    return ThirdParty(groups, texts)
}

/** Settings › About › Open-source licences: every library in the app; a tap shows its licence. */
@Composable
fun LicensesScreen(nav: Nav) {
    val c = LocalMc.current
    val context = LocalContext.current
    var data by remember { mutableStateOf<ThirdParty?>(null) }
    var open by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) { data = withContext(Dispatchers.IO) { runCatching { readThirdParty(context) }.getOrNull() } }
    Page(stringResource(R.string.licenses_title), nav) {
        Text(stringResource(R.string.licenses_intro), style = McType.small, color = c.muted)
        val d = data
        if (d == null) Text(stringResource(R.string.licenses_loading), style = McType.small, color = c.muted)
        else d.groups.forEach { (key, items) ->
            // "app" is MissionCtrl itself (GPL-3.0-or-later): no count, and the notice the GPL asks
            // for (copyright, no warranty, where to read the licence) stays visible under the entry.
            val isApp = key == "app"
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                SectionTitle(stringResource(groupTitle(key)) + if (isApp) "" else " · " + items.size)
                Block {
                    items.forEachIndexed { i, item ->
                        if (i > 0) Divider()
                        val id = key + "/" + item.name
                        Column(
                            Modifier.fillMaxWidth().clickable(role = Role.Button) { open = if (open == id) null else id }
                                .padding(horizontal = 14.dp, vertical = 10.dp),
                            verticalArrangement = Arrangement.spacedBy(2.dp),
                        ) {
                            Text(breakable(item.name) + if (item.version.isNotEmpty()) " " + item.version else "", style = McType.body, color = c.text)
                            Text(item.license + if (item.url.isNotEmpty()) " · " + item.url else "", style = McType.caption, color = c.muted)
                            if (isApp) Text(stringResource(R.string.licenses_app_notice), style = McType.small, color = c.muted, modifier = Modifier.padding(top = 6.dp))
                            if (open == id) item.texts.mapNotNull { d.texts[it] }.forEach {
                                Text(remember(it) { LicenseText.reflow(it) }, style = McType.path, color = c.mono, modifier = Modifier.padding(top = 8.dp))
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun groupTitle(key: String): Int = when (key) {
    "app" -> R.string.licenses_group_app
    "android" -> R.string.licenses_group_android
    "core" -> R.string.licenses_group_core
    "fonts" -> R.string.licenses_group_fonts
    else -> R.string.licenses_group_models
}

/** "androidx.compose.foundation:foundation-layout-android" may wrap after - : . (zero-width space), not inside a word. */
internal fun breakable(name: String): String = name.replace(Regex("([-:.])"), "$1\u200B")
