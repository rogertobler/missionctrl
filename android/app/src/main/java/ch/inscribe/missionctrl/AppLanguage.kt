package ch.inscribe.missionctrl

import android.app.Activity
import android.app.LocaleManager
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.os.Build
import android.os.LocaleList
import androidx.core.content.edit
import java.util.Locale

/**
 * The app's language: "" follows Android, else a tag from [available] (English in values/,
 * one values-xx/strings.xml per further language). From Android 13 Android keeps the choice
 * itself (and shows it in its app settings); below, our preferences and [wrap].
 */
object AppLanguage {
    private const val PREFS = "missionctrl"
    private const val KEY = "app_language"

    /** Tags the app has texts for, English first; generated from the res folders. */
    val available: List<String> = BuildConfig.APP_LOCALES.split(',').filter { it.isNotBlank() }

    /** A language's name in that language: "English", "Deutsch", "Français". */
    fun displayName(tag: String): String = Locale.forLanguageTag(tag).let { l -> l.getDisplayName(l).replaceFirstChar { it.titlecase(l) } }

    /** The chosen tag as one of [available] (region kept, "de-CH" from Android's settings counts as "de"); "" = System. */
    fun current(context: Context): String {
        val tag = if (Build.VERSION.SDK_INT >= 33) {
            context.getSystemService(LocaleManager::class.java)?.applicationLocales?.takeIf { !it.isEmpty }?.get(0)?.toLanguageTag().orEmpty()
        } else {
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, "").orEmpty()
        }
        if (tag.isEmpty()) return ""
        return available.firstOrNull { it.equals(tag, ignoreCase = true) }
            ?: available.firstOrNull { Locale.forLanguageTag(it).language == Locale.forLanguageTag(tag).language }
            ?: tag
    }

    /**
     * "" = follow Android. From Android 13 Android recreates the screens itself; below, the app
     * starts anew, so the service, notifications and the process-wide context speak it too.
     */
    fun set(activity: Activity, tag: String) {
        if (Build.VERSION.SDK_INT >= 33) {
            activity.getSystemService(LocaleManager::class.java)?.applicationLocales =
                if (tag.isEmpty()) LocaleList.getEmptyLocaleList() else LocaleList.forLanguageTags(tag)
        } else {
            // Written at once (commit): the process ends two lines further down.
            activity.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit(commit = true) { putString(KEY, tag) }
            val restart = activity.packageManager.getLaunchIntentForPackage(activity.packageName)
                ?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
            activity.startActivity(restart)
            Runtime.getRuntime().exit(0)
        }
    }

    /** Android 12 → 13 update: the choice moves from our preferences to Android's (once, at app start). */
    fun migrate(context: Context) {
        if (Build.VERSION.SDK_INT < 33) return
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val tag = prefs.getString(KEY, "").orEmpty()
        if (tag.isEmpty()) return
        val manager = context.getSystemService(LocaleManager::class.java) ?: return
        if (manager.applicationLocales.isEmpty) manager.applicationLocales = LocaleList.forLanguageTags(tag)
        prefs.edit { remove(KEY) }
    }

    /** Below Android 13: [base] in the chosen language (activity, service, application, receiver). */
    fun wrap(base: Context): Context {
        if (Build.VERSION.SDK_INT >= 33) return base
        val tag = base.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(KEY, "").orEmpty()
        if (tag.isEmpty()) return base
        val config = Configuration(base.resources.configuration).apply { setLocales(LocaleList.forLanguageTags(tag)) }
        return base.createConfigurationContext(config)
    }
}
