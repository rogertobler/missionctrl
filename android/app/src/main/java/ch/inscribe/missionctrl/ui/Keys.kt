package ch.inscribe.missionctrl.ui

import androidx.annotation.StringRes
import androidx.compose.ui.graphics.vector.ImageVector
import ch.inscribe.missionctrl.R

/** A configurable quick button. [keys] are herdr key names sent with pane.send_keys. */
data class QuickKey(
    val id: String,
    /** On the button (a word, or a sign like ↑ that stays in every language). */
    @StringRes val label: Int,
    /** For screen readers and the ⋯ sheet. */
    @StringRes val description: Int,
    val icon: ImageVector? = null,
    val keys: List<String> = emptyList(),
    /** Handled by the app instead of sending keys (voice, keyboard, paste …). */
    val special: Boolean = false,
    /** Only meaningful for free-text questions (hidden for menus). */
    val textOnly: Boolean = false,
    /** Only meaningful for menus (hidden for free-text questions). */
    val menuOnly: Boolean = false,
    /** Arms a modifier for the next key: "ctrl", "alt" or "ctrl+alt". */
    val mod: String? = null,
)

object QuickKeys {
    val all: List<QuickKey> = listOf(
        QuickKey("mic", R.string.key_mic_label, R.string.key_mic_desc, McIcons.Mic, special = true, textOnly = true),
        QuickKey("kb", R.string.key_kb_label, R.string.key_kb_desc, McIcons.Keyboard, special = true, textOnly = true),
        QuickKey("up", R.string.key_up_label, R.string.key_up_desc, McIcons.Up, listOf("up"), menuOnly = true),
        QuickKey("down", R.string.key_down_label, R.string.key_down_desc, McIcons.Down, listOf("down"), menuOnly = true),
        QuickKey("left", R.string.key_left_label, R.string.key_left_desc, McIcons.Left, listOf("left")),
        QuickKey("right", R.string.key_right_label, R.string.key_right_desc, McIcons.Right, listOf("right")),
        QuickKey("enter", R.string.key_enter_label, R.string.key_enter_desc, McIcons.Enter, listOf("enter")),
        QuickKey("stop", R.string.key_stop_label, R.string.key_stop_desc, McIcons.Stop, listOf("esc")),
        QuickKey("tab", R.string.key_tab_label, R.string.key_tab_desc, null, listOf("tab")),
        QuickKey("stab", R.string.key_stab_label, R.string.key_stab_desc, null, listOf("shift+tab")),
        QuickKey("ctrlc", R.string.key_ctrlc_label, R.string.key_ctrlc_desc, null, listOf("ctrl+c")),
        QuickKey("space", R.string.key_space_label, R.string.key_space_desc, null, listOf("space")),
        QuickKey("bksp", R.string.key_bksp_label, R.string.key_bksp_desc, null, listOf("backspace")),
        // herdr has no names for these; the core sends their xterm bytes (mc-core raw_key).
        QuickKey("home", R.string.key_home_label, R.string.key_home_desc, null, listOf("home")),
        QuickKey("end", R.string.key_end_label, R.string.key_end_desc, null, listOf("end")),
        QuickKey("pgup", R.string.key_pgup_label, R.string.key_pgup_desc, null, listOf("pageup")),
        QuickKey("pgdn", R.string.key_pgdn_label, R.string.key_pgdn_desc, null, listOf("pagedown")),
        QuickKey("del", R.string.key_del_label, R.string.key_del_desc, null, listOf("delete")),
        QuickKey("n1", R.string.key_n1_label, R.string.key_n1_desc, null, listOf("1"), menuOnly = true),
        QuickKey("n2", R.string.key_n2_label, R.string.key_n2_desc, null, listOf("2"), menuOnly = true),
        QuickKey("n3", R.string.key_n3_label, R.string.key_n3_desc, null, listOf("3"), menuOnly = true),
        QuickKey("paste", R.string.key_paste_label, R.string.key_paste_desc, McIcons.Paste, special = true),
        QuickKey("copy", R.string.key_copy_label, R.string.key_copy_desc, McIcons.Copy, special = true),
        QuickKey("file", R.string.key_file_label, R.string.key_file_desc, McIcons.Paperclip, special = true),
        QuickKey("ctrl", R.string.key_ctrl_label, R.string.key_ctrl_desc, special = true, mod = "ctrl"),
        QuickKey("alt", R.string.key_alt_label, R.string.key_alt_desc, special = true, mod = "alt"),
        QuickKey("ctrlalt", R.string.key_ctrlalt_label, R.string.key_ctrlalt_desc, special = true, mod = "ctrl+alt"),
        QuickKey("shift", R.string.key_shift_label, R.string.key_shift_desc, special = true, mod = "shift"),
    )

    private val byId = all.associateBy { it.id }

    fun get(id: String) = byId[id]

    /** The sheet behind ⋯: every key, in groups (name as a string resource). */
    val groups: List<Pair<Int, List<String>>> = listOf(
        R.string.keygroup_combine to listOf("ctrl", "alt", "ctrlalt", "shift"),
        R.string.keygroup_input to listOf("mic", "kb", "paste", "file", "copy"),
        R.string.keygroup_navigation to listOf("up", "down", "left", "right", "home", "end", "pgup", "pgdn", "tab", "stab"),
        R.string.keygroup_control to listOf("enter", "stop", "ctrlc", "bksp", "del", "space"),
        R.string.keygroup_menu to listOf("n1", "n2", "n3"),
    )

    /** The waiting card handles speech, typing, paste, modifiers and plain keys, not files or copying. */
    val availableStart = all.filter { !it.special || it.mod != null || it.id in setOf("mic", "kb", "paste") }.map { it.id }
    val availableDetail = all.map { it.id }
    /** Own buttons; in the terminal ☰ (without the dock) and ⋯ come on top of these, the card has neither. */
    const val MAX_START = 5
    const val MAX_DETAIL = 6
}

/** Ctrl/Alt for the next key: what the keyboard typed becomes a herdr key name. */
object ModKeys {
    /** herdr name of a key typed while a modifier was armed (bytes as [TermInputView] hands them over), or null. */
    fun name(typed: String): String? = when (typed) {
        "" -> null
        "\r" -> "enter"
        "\u007F" -> "backspace"
        "\t" -> "tab"
        "\u001B" -> "esc"
        "\u001B[A" -> "up"
        "\u001B[B" -> "down"
        "\u001B[C" -> "right"
        "\u001B[D" -> "left"
        "\u001B[H" -> "home"
        "\u001B[F" -> "end"
        "\u001B[3~" -> "delete"
        "\u001B[5~" -> "pageup"
        "\u001B[6~" -> "pagedown"
        " " -> "space"
        "+" -> "plus"
        // herdr reads an upper-case letter as Shift + letter.
        else -> String(Character.toChars(typed.codePointAt(0))).takeUnless { it[0] == '\u001B' }
    }

    /** The chord for [mod] and a bar key's single herdr key, e.g. "ctrl" + "left" → "ctrl+left". */
    fun chord(mod: String, key: String) = "$mod+$key"
}

enum class QuestionKind { MENU, TEXT, UNKNOWN }

data class MenuOption(val number: Int, val title: String, val detail: String, val selected: Boolean)
data class Menu(val question: String, val options: List<MenuOption>)

object PaneTextTools {
    private val menuLine = Regex("""^\s*(?:[❯›>▸]\s*)?\d+[.)]\s+\S""")
    private val chromeOnly = Regex("""^[\s╭╮╰╯─│┃━┌┐└┘├┤┬┴┼═║╔╗╚╝]*$""")
    /** Frames and separators, also those with a label on them (Claude's "──── Session ─"). */
    private object chrome { fun matches(line: String) = chromeOnly.matches(line) || Reflow.isSeparator(line) }

    /** Remove box-drawing borders that TUIs draw around input areas. */
    fun clean(line: String): String = line.trimEnd()
        .removePrefix("│").removeSuffix("│").removePrefix("┃").removeSuffix("┃")
        .trimEnd()

    fun lines(text: String): List<String> = text.lines().map(::clean)

    /** The tail of a pane that is worth showing as "the question". */
    fun question(text: String, maxLines: Int = 8): String {
        // Blank lines stay for now: they mark where the question above a menu begins. Frames and
        // separators become blank lines too (Claude's "╌╌╌" between a diff and its question).
        val all = lines(text)
        val kept = all.map { if (it.isNotBlank() && chrome.matches(it)) "" else it }
        // Drop the agent's empty input prompt line and hints at the very bottom.
        fun bottom(l: String) = l.isBlank() || l.trim().let { t -> t == ">" || t == "❯" || statusLine.containsMatchIn(t) || menuFooter.containsMatchIn(t) }
        var trimmed = kept.dropLastWhile(::bottom)
        // On a narrow pane the footer wraps ("Enter to select · … · Esc to" / "cancel"): its last
        // one or two lines are no footer of their own, but go with the footer line above them.
        val wrapped = trimmed.indices.reversed().take(3).firstOrNull { menuFooter.containsMatchIn(trimmed[it].trim()) }
        if (wrapped != null && trimmed.drop(wrapped + 1).none { menuLine.containsMatchIn(it) }) trimmed = trimmed.take(wrapped).dropLastWhile(::bottom)
        // A choice menu is shown whole, from the question above option 1 to the last option,
        // however long: cut off at the top, the first options were missing.
        val first = trimmed.indexOfLast { firstOption.containsMatchIn(it) }
        if (first >= 0 && trimmed.drop(first).count { menuLine.containsMatchIn(it) } >= 2) {
            var start = first
            while (start > 0 && trimmed[start - 1].isBlank()) start--
            // The question is the paragraph right above the options.
            while (start > 0 && trimmed[start - 1].isNotBlank() && first - start < 6) start--
            // Claude's plan question: the plan's last lines stand right above it, the question starts at Claude's line.
            (start until first).firstOrNull { planQuestion.containsMatchIn(trimmed[it]) }?.let { start = it }
            // Codex puts details ("Description: …") between its question and the options: a paragraph
            // ending in "?" right above them belongs to the question.
            if ((start until first).none { "?" in trimmed[it] }) {
                var gap = start
                while (gap > 0 && trimmed[gap - 1].isBlank()) gap--
                var above = gap
                while (above > 0 && trimmed[above - 1].isNotBlank() && gap - above < 3) above--
                if (above < gap && trimmed[gap - 1].trimEnd().endsWith("?")) start = above
            }
            val block = trimmed.drop(start).map { it.replace(questionPrefix, "") }.filterNot { it.isBlank() }
                .mapIndexed { i, l -> if (i == 0) l.replace(scrollMark, "") else l }
            val width = all.filter { it.isNotBlank() && chrome.matches(it) }.maxOfOrNull { it.trimEnd().length }
            return (claudeCommand(all, block) + joinWrapped(block, width)).joinToString("\n")
        }
        // A menu without numbers ("❯ No, exit / Yes, I trust this folder" – the trust questions of
        // Claude and agy): only right above a menu footer; numbered here, so the rest reads it as one.
        if (kept.size > trimmed.size && kept.drop(trimmed.size).any { menuFooter.containsMatchIn(it.trim()) }) {
            unnumbered(trimmed)?.let { return it }
        }
        return trimmed.filterNot { it.isBlank() }.takeLast(maxLines).joinToString("\n")
    }

    /** Lines at the bottom that form a menu without numbers, as "❯ 1. …" lines under their question. */
    private fun unnumbered(lines: List<String>): String? {
        val marked = lines.indexOfLast { unnumberedMark.containsMatchIn(it) }
        if (marked < 0) return null
        val column = unnumberedMark.find(lines[marked])!!.range.last + 1
        fun belongs(l: String) = l.isNotBlank() && (unnumberedMark.containsMatchIn(l) ||
            (l.length > column && l.take(column).isBlank() && !l[column].isWhitespace()))
        var first = marked
        while (first > 0 && belongs(lines[first - 1])) first--
        var last = marked
        while (last < lines.lastIndex && belongs(lines[last + 1])) last++
        if (last != lines.lastIndex || last - first < 1) return null
        val options = (first..last).map { i -> (if (i == marked) "❯ " else "  ") + "${i - first + 1}. " + lines[i].drop(column).trim() }
        // The question: the nearest paragraph above, or one with a "?" up to three paragraphs higher.
        var start = first
        var paragraphs = 0
        var ask = first
        while (start > 0 && paragraphs < 3) {
            while (start > 0 && lines[start - 1].isBlank()) start--
            var top = start
            while (top > 0 && lines[top - 1].isNotBlank() && start - top < 6) top--
            if (paragraphs == 0) ask = top
            if ((top until start).any { "?" in lines[it] }) { ask = top; break }
            start = top
            paragraphs++
        }
        return (lines.subList(ask, first).filterNot { it.isBlank() } + options).joinToString("\n")
    }

    /**
     * Claude asks "Do you want to proceed?" under the command it wants to run, between two "╌╌╌"
     * lines, with what the command is for above them: both go above the question, as "$ command".
     */
    private fun claudeCommand(all: List<String>, block: List<String>): List<String> {
        if (block.firstOrNull()?.trim() != CLAUDE_PROCEED) return emptyList()
        val ask = all.indexOfLast { it.trim() == CLAUDE_PROCEED }
        val dashed = (0 until ask).filter { dashedLine.matches(all[it]) }.takeLast(2)
        if (dashed.size < 2 || ask - dashed[1] > 6) return emptyList()
        val command = all.subList(dashed[0] + 1, dashed[1]).map { it.trim().removePrefix("│").trim() }.filter { it.isNotEmpty() }
        if (command.isEmpty()) return emptyList()
        val purpose = all.subList(0, dashed[0]).lastOrNull { it.isNotBlank() }?.trim()
            ?.takeUnless { it.startsWith("Tip:") || it == "Bash command" || chrome.matches(it) }
        return listOfNotNull(purpose) + ("$ " + command.take(3).joinToString(" ") + if (command.size > 3) " …" else "")
    }

    private const val CLAUDE_PROCEED = "Do you want to proceed?"
    private val dashedLine = Regex("""^\s*╌{10,}\s*$""")

    /**
     * Option titles the agent wrapped onto the next line, back on one line: the next line starts
     * at the title's column and its first word would not have fit on the option's line ([width]
     * from the screen's full-width separators; without them, as it is).
     */
    private fun joinWrapped(block: List<String>, width: Int?): List<String> {
        if (width == null) return block
        val out = mutableListOf<String>()
        var physical: String? = null // the line as the agent drew it, before joining
        for (line in block) {
            val option = physical?.let { optionLine.find(it) ?: optionLine.find(out.last()) }
            val word = line.trim().substringBefore(' ')
            if (physical != null && option != null && !menuLine.containsMatchIn(line) &&
                line.length - line.trimStart().length == option.groups[2]!!.range.first &&
                physical.trimEnd().length + 1 + word.length > width - 2
            ) out[out.lastIndex] = out.last().trimEnd() + " " + line.trim()
            else out += line
            physical = line
        }
        return out
    }

    /** Claude's mode line under its input ("⏸ manual mode on · ? for shortcuts") and its effort mark. */
    private val statusLine = Regex("""\? for shortcuts|shift\+tab to cycle|· /effort$""")

    /** The cursor of a menu without numbers: "❯ text", "> text", "› text". */
    private val unnumberedMark = Regex("""^\s*[❯›>▸] (?=\S)""")
    /** "Question 1/1: " before the question itself (agy), alone on its line at Codex. */
    private val questionPrefix = Regex("""^\s*Question\s+\d+/\d+(?:\s+\(\d+ unanswered\))?:?\s*""")
    /** Markdown links in agents' text: "[calc.py](file:///…)" → "calc.py". */
    private val markdownLink = Regex("""\[([^\]]+)\]\([^)\s]+\)""")

    /**
     * Joins lines the agent wrapped at the laptop's width back into paragraphs (same rules as
     * the terminal view, [Reflow.continues]), so the phone wraps them once, at its own width.
     */
    fun unwrap(question: String): String {
        val lines = question.lines()
        val paneCols = lines.maxOfOrNull { it.trimEnd().length } ?: 0
        val out = ArrayList<String>(lines.size)
        for (line in lines) {
            val last = out.lastOrNull()
            if (last != null && Reflow.continues(last, line, paneCols)) out[out.lastIndex] = last.trimEnd() + " " + line.trim()
            else out += line
        }
        return out.joinToString("\n")
    }

    /**
     * A choice menu split into its question and options (title, the indented lines under it,
     * and which one the agent's cursor is on), or null when [question] holds no menu.
     */
    fun menu(question: String): Menu? {
        val lines = question.lines()
        val first = lines.indexOfFirst { optionLine.containsMatchIn(clean(it)) }
        if (first < 0) return null
        val options = mutableListOf<MenuOption>()
        for (line in lines.drop(first)) {
            val m = optionLine.find(clean(line))
            if (m != null) {
                val selected = clean(line).trimStart().firstOrNull()?.let { it in "❯›>▸" } == true
                val (title, detail) = splitOption(m.groupValues[2].replace(markdownLink, "$1"))
                options += MenuOption(m.groupValues[1].toInt(), title, detail, selected)
            } else if (line.isNotBlank() && options.isNotEmpty()) {
                val o = options.last()
                options[options.lastIndex] = o.copy(detail = (o.detail + " " + line.trim()).trim())
            }
        }
        if (options.size < 2) return null
        // One line, apart from commands ("$ python3 calc.py"): they stand on their own.
        val ask = unwrap(lines.take(first).joinToString("\n")).lines().map { it.trim() }.filter { it.isNotEmpty() }
            .fold("") { acc, l -> if (acc.isEmpty()) l else if (l.startsWith("$ ") || acc.substringAfterLast('\n').startsWith("$ ")) "$acc\n$l" else "$acc $l" }
            .replace(markdownLink, "$1")
        return Menu(ask, options.distinctBy { it.number })
    }

    /**
     * An option line's text as title and explanation: Codex puts both in one line, apart by a column
     * gap ("Subcommands (Recommended)  Use `calc.py add 2 3`"); its shortcut ("(y)", "(esc)") goes.
     */
    private fun splitOption(text: String): Pair<String, String> {
        val parts = text.split(columnGap, limit = 2)
        return parts[0].replace(shortcutSuffix, "").trim() to parts.getOrElse(1) { "" }.trim()
    }

    private val columnGap = Regex("\\s{2,}")
    private val shortcutSuffix = Regex("""\s+\((?:[a-z]|esc)\)$""")

    /**
     * Keys for [picked] against the screen as it is now: the cursor may have
     * moved on the laptop, or another question may be open. Null if that option is no longer there.
     */
    fun chooseNow(screen: String, picked: MenuOption): List<String>? {
        val now = menu(question(screen)) ?: return null
        val target = now.options.firstOrNull { it.number == picked.number && it.title == picked.title } ?: return null
        return choose(now, target)
    }

    /** Whether the screen still offers option [number] with [title] (answers from a notification). */
    fun stillOffers(screen: String, number: String, title: String): Boolean =
        options(question(screen)).any { (n, t) -> n == number && t == title }

    /** What identifies a question: answered, its card waits for the next one. */
    fun signature(question: String): String =
        menu(question)?.let { m -> m.question + "|" + m.options.joinToString("|") { "${it.number}.${it.title}" } } ?: question.trim()

    /**
     * Claude's tabs over a series of questions ("☒ Telegram ☐ Tags ☐ Versand ✔ Submit"): answered
     * and total; null for a single question.
     */
    fun progress(screen: String): Pair<Int, Int>? {
        // Codex: "Question 2/2 (1 unanswered)" above its question (not "• Questions 2/2 answered" afterwards).
        lines(screen).lastOrNull { codexQuestion.containsMatchIn(it) }?.let { line ->
            val m = codexQuestion.find(line)!!
            val (current, total) = m.groupValues[1].toInt() to m.groupValues[2].toInt()
            val open = m.groupValues[3].toIntOrNull()
            return if (total >= 2) (if (open != null) total - open else current - 1) to total else null
        }
        val tabs = lines(screen).lastOrNull { "Submit" in it && ('☐' in it || '☒' in it) } ?: return null
        val done = tabs.count { it == '☒' }
        val total = done + tabs.count { it == '☐' }
        return if (total >= 2) done to total else null
    }

    private val codexQuestion = Regex("""^\s*Question\s+(\d+)/(\d+)(?:\s+\((\d+) unanswered\))?""")

    /**
     * Claude's "Review your answers" before Submit: question → answer, in order (wrapped lines
     * joined); null on any other screen.
     */
    fun review(screen: String): List<Pair<String, String>>? {
        val all = lines(screen)
        val start = all.indexOfLast { it.trim() == "Review your answers" }
        if (start < 0) return null
        val end = all.drop(start + 1).indexOfFirst { it.trim().startsWith("Ready to submit") }.let { if (it < 0) all.size else start + 1 + it }
        val out = mutableListOf<Pair<String, String>>()
        var question: String? = null
        var answer: String? = null
        fun flush() {
            question?.let { out += it to answer.orEmpty() }
            question = null; answer = null
        }
        for (raw in all.subList(start + 1, end)) {
            val line = raw.trim().trimStart('│', '┃', '|').trim()
            when {
                line.isEmpty() -> Unit
                line.startsWith("●") || line.startsWith("•") -> { flush(); question = line.drop(1).trim() }
                line.startsWith("→") -> answer = line.drop(1).trim()
                answer != null -> answer = "$answer $line"
                question != null -> question = "$question $line"
            }
        }
        flush()
        return out.takeIf { it.isNotEmpty() }
    }

    /** Claude Code's free-text rows: "4. Type something." and, for a plan, "3. Tell Claude what to change". */
    private val freeTextTitle = Regex("""^(type something|tell claude what to change)\b""", RegexOption.IGNORE_CASE)

    private val planQuestion = Regex("""has written up a plan""")
    /** Claude's mark that the box above scrolls ("↓ Claude has written up a plan …"). */
    private val scrollMark = Regex("""^\s*[↓↑]\s*""")
    private val planFile = Regex("""~?/[A-Za-z0-9._/~-]*/plans/[A-Za-z0-9._-]+\.md""")

    /** Path of the plan Claude asks about, as shown under its options; read with Core.readPlan. */
    fun planPath(screen: String): String? =
        if (planQuestion.containsMatchIn(screen)) planFile.findAll(screen).lastOrNull()?.value else null

    /** "5. Chat about this": closes the question so the agent and you can talk about it. */
    private val chatTitle = Regex("""^chat about this\b""", RegexOption.IGNORE_CASE)
    /**
     * Codex: "No, and tell Codex what to do differently" (an approval) and "No, stay in Plan mode"
     * close the question; what you write then is a new message (seen with codex-cli 0.163).
     */
    private val messageTitle = Regex("""^(no, and tell codex what to do differently|no, stay in plan mode)\b""", RegexOption.IGNORE_CASE)
    /** Codex's questions: "None of the above", text goes into its notes field (Tab). */
    private val noteTitle = Regex("""^none of the above\b""", RegexOption.IGNORE_CASE)
    /** agy's free-text row "4. Write-in...": Enter opens "Your answer:", then the text. */
    private val writeInTitle = Regex("""^write-in\b""", RegexOption.IGNORE_CASE)
    private val writeInOpen = Regex("""(?im)^\s*your answer:""")

    /** Codex's notes field is open ("tab or esc to clear notes"). */
    private val notesOpen = Regex("""(?i)tab or esc to clear notes""")

    fun freeText(menu: Menu?): MenuOption? = menu?.options?.firstOrNull { freeTextTitle.containsMatchIn(it.title) }

    fun chat(menu: Menu?): MenuOption? = menu?.options?.firstOrNull { chatTitle.containsMatchIn(it.title) }

    fun message(menu: Menu?): MenuOption? = menu?.options?.firstOrNull { messageTitle.containsMatchIn(it.title) }

    fun writeIn(menu: Menu?): MenuOption? = menu?.options?.firstOrNull { writeInTitle.containsMatchIn(it.title) }

    fun note(menu: Menu?): MenuOption? = menu?.options?.firstOrNull { noteTitle.containsMatchIn(it.title) && "notes" in it.detail }

    /** Rows that want typed text: free-text and notes rows, "Chat about this", Codex's "No, and tell …". */
    fun takesText(menu: Menu?, option: MenuOption?): Boolean =
        option != null && (option == freeText(menu) || option == chat(menu) || option == message(menu) || option == note(menu) || option == writeIn(menu))

    /** After [key] ("up"/"down") from the cursor now, does it land on a row that wants text? */
    fun landsOnText(menu: Menu?, key: String?): Boolean {
        val options = menu?.options ?: return false
        val from = options.indexOfFirst { it.selected }.coerceAtLeast(0)
        val to = when (key) { "down" -> from + 1; "up" -> from - 1; else -> return false }
        return takesText(menu, options.getOrNull(to))
    }

    /**
     * CHAT: the question closes, the text is a chat message, the card opens the terminal.
     * MESSAGE: the same without the terminal (a correction to Codex). FREE_TEXT: into Claude's
     * text row. NOTE: into Codex's notes field of "None of the above". PLAIN: as it is.
     */
    enum class TextRoute { PLAIN, FREE_TEXT, CHAT, MESSAGE, NOTE }

    /**
     * Where typed text goes as the screen is now, and the keys before it: cursor on "Chat about
     * this" or Codex's "No, and tell …" → pick it, then the text is a message; Claude's free-text
     * row → cursor there; Codex's "None of the above" → cursor there and Tab for its notes;
     * otherwise plain input.
     */
    fun route(screen: String): Pair<TextRoute, List<String>> {
        val menu = menu(question(screen)) ?: return TextRoute.PLAIN to emptyList()
        chat(menu)?.takeIf { it.selected }?.let { return TextRoute.CHAT to listOf("enter") }
        message(menu)?.takeIf { it.selected }?.let { return TextRoute.MESSAGE to listOf("enter") }
        freeText(menu)?.let { return TextRoute.FREE_TEXT to choose(menu, it).dropLast(1) }
        note(menu)?.let { row ->
            val open = row.selected && notesOpen.containsMatchIn(screen)
            return TextRoute.NOTE to if (open) emptyList() else choose(menu, row).dropLast(1) + "tab"
        }
        // agy: the row with Enter opens its answer field; open already, just the text.
        writeIn(menu)?.let { row ->
            val open = row.selected && writeInOpen.containsMatchIn(screen)
            return TextRoute.MESSAGE to if (open) emptyList() else choose(menu, row)
        }
        return TextRoute.PLAIN to emptyList()
    }

    /** Codex's plan question ("Implement this plan?"); the plan stands above it in the pane. */
    fun asksToImplementPlan(screen: String): Boolean = implementPlan.containsMatchIn(screen)

    private val implementPlan = Regex("""(?m)^\s*Implement this plan\?\s*$""")

    /**
     * The plan Codex wrote into the pane: the lines after its last "• Proposed Plan" up to "Worked
     * for …" or "Implement this plan?", indentation removed; null without one.
     */
    fun inlinePlan(text: String): String? {
        val all = lines(text)
        val start = all.indexOfLast { it.trim() == "• Proposed Plan" }
        if (start < 0) return null
        val body = all.drop(start + 1).takeWhile { !it.trimStart().startsWith("Worked for") && !implementPlan.matches(it) }
        val indent = body.filter { it.isNotBlank() }.minOfOrNull { l -> l.takeWhile { it == ' ' }.length } ?: 0
        return body.joinToString("\n") { it.drop(indent) }.trim().ifEmpty { null }
    }

    fun choose(menu: Menu, target: MenuOption): List<String> {
        val from = menu.options.indexOfFirst { it.selected }.coerceAtLeast(0)
        val to = menu.options.indexOf(target)
        val step = if (to > from) "down" else "up"
        return List(kotlin.math.abs(to - from)) { step } + "enter"
    }

    /** Option 1 of a menu, selected or not. */
    private val firstOption = Regex("""^\s*(?:[❯›>▸]\s*)?1[.)]\s+\S""")
    /**
     * Key hints under a menu: Claude's and Codex's (approval, question, trust and plan dialogs).
     * Which Codex dialogs exist: herdr's src/detect/manifests/codex.toml (Apache-2.0, herdr 0.9.5);
     * their exact texts: our recordings with codex-cli 0.163 (test resources screens/codex/).
     */
    private val menuFooter = Regex(
        """(?i)^(enter to select|enter to confirm|esc to cancel|press enter to confirm|enter (continue|select|confirm|submit) ·|↑/↓ navigate|tab to add notes|tab or esc to clear notes|enter to submit|ctrl\+g to edit|shift\+tab to approve)""",
    )

    /** Selected option of a choice menu, e.g. "❯ 3. Tell Claude what to change". */
    private val selectedMenuLine = Regex("""^\s*[❯›▸]\s*\d+[.)]\s+\S""")

    /**
     * One line for a collapsed notification: a menu's question (its last line, after a command), not
     * the last option ("5. Chat about this"); otherwise the last line.
     */
    fun headline(question: String): String? =
        menu(question)?.question?.lineSequence()?.lastOrNull { it.isNotBlank() }?.trim()
            ?: question.lineSequence().lastOrNull { it.isNotBlank() }?.trim()

    /** The bottom of a screen shows a choice menu waiting for an answer. */
    fun showsMenu(screen: String): Boolean {
        val bottom = lines(screen).filterNot { it.isBlank() }.takeLast(20)
        return bottom.any { selectedMenuLine.containsMatchIn(it) } && bottom.count { menuLine.containsMatchIn(it) } >= 2
    }

    fun kind(question: String): QuestionKind {
        val menu = question.lines().count { menuLine.containsMatchIn(it) }
        return when {
            menu >= 2 -> QuestionKind.MENU
            question.isNotBlank() -> QuestionKind.TEXT
            else -> QuestionKind.UNKNOWN
        }
    }

    private val optionLine = Regex("""^\s*(?:[❯›>▸]\s*)?([1-9])[.)]\s+(.+?)\s*$""")

    /** Numbered options of a choice menu, e.g. "1" to "Yes", in screen order. */
    fun options(question: String): List<Pair<String, String>> =
        question.lines().mapNotNull { optionLine.find(clean(it))?.destructured?.let { (n, t) -> n to splitOption(t).first } }.distinctBy { it.first }

    /** Claude's and Codex's working line: "✻ Type-checking… (13m 10s · ↓ 285k tokens · esc to interrupt)". */
    private val spinnerLine = Regex("""^\s*[✻✶✳✢✽✺*·•\u2800-\u28FF]\s+(\S.*)$""")
    private val interruptHint = Regex("""\s*[·•]?\s*esc to interrupt\)?""")

    /**
     * One line on what the agent is doing, for the list: while it works its spinner line
     * ("Type-checking… 13m 10s · ↓ 285k tokens"), at rest the start of its last answer.
     */
    fun activity(screen: String, working: Boolean): String? {
        val all = lines(screen).filterNot { it.isBlank() || chrome.matches(it) }
        if (working) {
            // Claude's spinner glyphs always; a plain bullet only with the interrupt hint (Codex),
            // since lists in the output use bullets too.
            val line = all.takeLast(12).lastOrNull {
                spinnerLine.matches(it) && it.trimStart().first().let { g -> g in "✻✶✳✢✽✺·" || g in '\u2801'..'\u28FF' } ||
                    spinnerLine.matches(it) && it.contains("esc to interrupt")
            } ?: return null
            val text = spinnerLine.find(line)!!.groupValues[1]
            return text.replace(interruptHint, "").replace("(", "").replace(")", "").trim().ifEmpty { null }
        }
        // Only a real answer: without its marker the screen shows something else (Claude's
        // start banner with its block-letter logo, a shell), which says nothing about the work.
        val text = lines(screen).filterNot { it.isNotBlank() && chrome.matches(it) }
        val start = text.indexOfLast { isAnswerStart(it) }
        // agy: after "▸ Thought for 3s" comes its thinking in one line ("…"), then the answer.
        if (start >= 0 && thoughtLine.containsMatchIn(text[start])) {
            val rest = text.drop(start + 1).filter { it.isNotBlank() }
            val answer = rest.drop(if (rest.firstOrNull()?.trimEnd()?.endsWith("...") == true) 1 else 0).firstOrNull()
            return answer?.trim()?.trimStart('#', ' ')?.replace(markdownLink, "$1")?.ifEmpty { null }
        }
        // A start banner with a logo and text (Claude, agy) says nothing about the work.
        if (start < 0 && text.any { blockGlyphs.containsMatchIn(it) && it.replace(blockGlyphs, "").count(Char::isLetter) >= 3 }) return null
        // Marker scrolled off the screen: the first line of what is left, but never a logo or
        // the prompt.
        // Codex's banner ">_ OpenAI Codex (v0.163.0)" counts, its prompt "›" does not.
        if (start < 0) return text.takeLast(40).firstOrNull {
            it.isNotBlank() && !blockGlyphs.containsMatchIn(it) && it.trimStart().let { t -> t.startsWith(">_") || t.first() !in ">❯›" }
        }?.trim()?.removePrefix(">_")?.removePrefix("⎿")?.trim()
        // The first paragraph, joined: a long "⏺ Bash(cd …)" is wrapped over several lines.
        val paragraph = listOf(text[start]) + text.drop(start + 1)
            .takeWhile { it.isNotBlank() && !isAnswerStart(it) && !it.trimStart().startsWith("⎿") }.take(3)
        return paragraph.joinToString(" ") { it.trim() }.replace(markdownLink, "$1").replace(expandHint, "")
            .removePrefix("⏺").removePrefix("●").removePrefix("•")
            .replace(blockGlyphs, "").replace(Regex("\\s+"), " ").trim().ifEmpty { null }
    }

    /** A marker at the left edge (indented ones are list bullets in the text, e.g. in agy's plans). */
    private fun isAnswerStart(line: String) = line.length - line.trimStart().length <= 1 &&
        line.trimStart().let { it.startsWith("⏺") || it.startsWith("●") || it.startsWith("•") || thoughtLine.containsMatchIn(it) }

    /** agy's summary of its thinking: "▸ Thought for 3s, 198 tokens", "○ Thought for 6.3s (…)". */
    private val thoughtLine = Regex("""^\s*[▸○] Thought for """)
    private val expandHint = Regex("""\s*\(ctrl\+o to expand\)""")

    /** Block elements (logos, bars) that mean nothing in one line of text. */
    private val blockGlyphs = Regex("[\\u2580-\\u259F\\u2800-\\u28FF]")

    /**
     * Last agent message, as plain text: from the last "⏺"/"●"/"•" marker up to what the agent shows
     * after it (Claude's "✻ Brewed for 5s · done", its input box and mode line). The marker goes, and
     * the hanging indent under it.
     */
    fun lastAnswer(text: String): String {
        // Borders are cleaned off only to find where the answer ends; the copy keeps tables whole.
        val raw = text.lines().map { it.trimEnd() }
        val all = raw.map(::clean)
        val start = all.indexOfLast { answerMarker.containsMatchIn(it) }
        val from = if (start >= 0) start else maxOf(0, all.size - 40)
        val end = (from + 1 until all.size).firstOrNull { afterAnswer(all, it) } ?: all.size
        // The top of an older, framed input box ("╭───╮" over "│ > │") is no part of it either.
        val block = raw.subList(from, end).dropLastWhile { it.isBlank() || it.trimStart().startsWith("╭") }
        if (block.isEmpty()) return ""
        if (start < 0) return block.joinToString("\n").trim()
        val marker = answerMarker.find(block.first())!!.value.length
        val first = block.first().drop(marker)
        // Continuation lines hang under the text, not under the marker: drop that much indent, no more.
        val rest = block.drop(1).map { l -> l.drop(minOf(marker, l.length - l.trimStart().length)) }
        return (listOf(first) + rest).joinToString("\n").trim()
    }

    /** At the left edge only: an indented "•" is a list in the answer (agy). */
    private val answerMarker = Regex("""^ ?[⏺●•][\s\u00a0]+""")
    /** Claude's summary under a finished turn ("✻ Cooked for 8s · done 17:43") and its working line. */
    private val turnLine = Regex("""^\s*[✻✶✳✢✽✺]\s+\S""")
    private val promptLine = Regex("""^\s*[❯>]([ \u00a0]|$)""")
    /**
     * First line that is no longer the answer: the turn line, the input box (a separator with the
     * prompt under it; a separator alone may be a table row), its mode line, a menu's keys.
     */
    private fun afterAnswer(lines: List<String>, i: Int): Boolean {
        val line = lines[i]
        val t = line.trim()
        if (turnLine.containsMatchIn(line) || t == ">" || t == "❯" || statusLine.containsMatchIn(t) || menuFooter.containsMatchIn(t)) return true
        return Reflow.isSeparator(line) && lines.drop(i + 1).firstOrNull { it.isNotBlank() }?.let { promptLine.containsMatchIn(it) } == true
    }

    /** Keys of [ids] that make sense for a question of [kind]. */
    /** Menus hide speech and keyboard; their text rows open the field when picked. */
    fun fitting(ids: List<String>, kind: QuestionKind): List<QuickKey> = ids.mapNotNull(QuickKeys::get).filter {
        when (kind) {
            QuestionKind.MENU -> !it.textOnly
            QuestionKind.TEXT -> !it.menuOnly
            QuestionKind.UNKNOWN -> true
        }
    }
}
