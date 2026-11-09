package ch.inscribe.missionctrl.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/**
 * Stroke icons on a 24×24 grid, drawn from the same paths as the design
 * prototype. Tinted at use site; no icon library needed.
 */
object McIcons {
    private fun stroke(name: String, width: Float = 1.7f, vararg paths: String, filled: Boolean = false): ImageVector {
        val b = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        for (p in paths) {
            b.addPath(
                pathData = addPathNodes(p),
                fill = if (filled) SolidColor(Color.Black) else null,
                stroke = if (filled) null else SolidColor(Color.Black),
                strokeLineWidth = width,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        return b.build()
    }

    private fun rect(x: Float, y: Float, w: Float, h: Float, r: Float) =
        "M${x + r} $y H${x + w - r} A$r $r 0 0 1 ${x + w} ${y + r} V${y + h - r} A$r $r 0 0 1 ${x + w - r} ${y + h} " +
            "H${x + r} A$r $r 0 0 1 $x ${y + h - r} V${y + r} A$r $r 0 0 1 ${x + r} $y Z"

    private fun circle(cx: Float, cy: Float, r: Float) =
        "M${cx - r} $cy A$r $r 0 1 0 ${cx + r} $cy A$r $r 0 1 0 ${cx - r} $cy Z"

    val Mic = stroke("mic", 1.7f, rect(9f, 3f, 6f, 11f, 3f), "M5 11a7 7 0 0 0 14 0M12 18v3")
    val Keyboard = stroke("keyboard", 1.7f, rect(2.5f, 6f, 19f, 12f, 2f), "M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10")
    val Up = stroke("up", 1.7f, "M12 19V5M6 11l6-6 6 6")
    val Down = stroke("down", 1.7f, "M12 5v14M6 13l6 6 6-6")
    val Left = stroke("left", 1.7f, "M19 12H5M11 6l-6 6 6 6")
    val Right = stroke("right", 1.7f, "M5 12h14M13 6l6 6-6 6")
    val Enter = stroke("enter", 2.1f, "M20 5v7a3 3 0 0 1-3 3H5M9 11l-4 4 4 4")
    /** ➤ Send in the reply field: Enter there makes a new line, so ↵ would mislead. */
    val Send = stroke("send", 1.9f, "M4.5 4.5L20 12L4.5 19.5L8 12Z")
    val Stop = stroke("stop", 1f, rect(5f, 5f, 14f, 14f, 2f), filled = true)
    val Close = stroke("close", 1.8f, "M6 6l12 12M18 6L6 18")
    val Menu = stroke("menu", 1.8f, "M4 7h16M4 12h16M4 17h16")
    val Check = stroke("check", 2f, "M5 12.5l4.5 4.5L19 7.5")
    /** Status "working": upright, swung by [StatusIcon]. */
    val Hammer = stroke("hammer", 1.8f, rect(5f, 3.5f, 14f, 5.5f, 1.5f), "M12 9v12")
    /** Status "waiting for you". */
    val Question = stroke("question", 1.8f, circle(12f, 12f, 9f), "M9.4 9.4a2.6 2.6 0 1 1 3.7 2.4c-.7.3-1.1.9-1.1 1.7v.5", "M12 16.9v.2")
    val Trash = stroke("trash", 1.7f, "M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3")
    val Back = stroke("back", 1.8f, "M15 6l-6 6 6 6")
    val Chevron = stroke("chevron", 1.8f, "M9 6l6 6-6 6")
    val ChevronDown = stroke("chevronDown", 2f, "M6 9l6 6 6-6")
    val Plus = stroke("plus", 1.8f, "M12 5v14M5 12h14")
    val Minus = stroke("minus", 1.8f, "M5 12h14")
    val Pencil = stroke("pencil", 1.7f, "M4 20h4L19 9l-4-4L4 16z", "M13.5 6.5l4 4")
    val Sliders = stroke("sliders", 1.7f, "M4 7h10M18 7h2M4 17h2M10 17h10", circle(16f, 7f, 2f), circle(8f, 17f, 2f))
    val Pin = stroke("pin", 1.7f, "M9 4h6l-1 5 3 3v2H7v-2l3-3z", "M12 14v6")
    /** Pinned: head filled, needle still drawn as a line. */
    val PinFilled: ImageVector = ImageVector.Builder("pinFilled", 24.dp, 24.dp, 24f, 24f)
        .addPath(
            pathData = addPathNodes("M9 4h6l-1 5 3 3v2H7v-2l3-3z"),
            fill = SolidColor(Color.Black), stroke = SolidColor(Color.Black),
            strokeLineWidth = 1.7f, strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round,
        )
        .addPath(
            pathData = addPathNodes("M12 14v6"),
            stroke = SolidColor(Color.Black), strokeLineWidth = 1.9f, strokeLineCap = StrokeCap.Round,
        )
        .build()
    val Unpin = stroke("unpin", 1.7f, "M9 4h6l-1 5 3 3v2H7v-2l3-3z", "M12 14v6M4 4l16 16")
    val Open = stroke("open", 1.7f, "M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5")
    val Settings = stroke(
        "settings", 1.6f, circle(12f, 12f, 3f),
        "M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
    )
    val Mission = stroke("mission", 1.7f, circle(12f, 12f, 9f), circle(12f, 12f, 4f), "M12 3v3M12 18v3M3 12h3M18 12h3")
    val Grid = stroke("grid", 1.7f, rect(3.5f, 3.5f, 7f, 7f, 1.5f), rect(13.5f, 3.5f, 7f, 7f, 1.5f), rect(3.5f, 13.5f, 7f, 7f, 1.5f), rect(13.5f, 13.5f, 7f, 7f, 1.5f))
    val Terminal = stroke("terminal", 1.7f, rect(3.5f, 4f, 17f, 16f, 2f), "M7.5 9l3 3-3 3M12.5 15h4")
    val PanelCollapse = stroke("panelCollapse", 1.7f, rect(3.5f, 4f, 17f, 16f, 2f), "M9.5 4v16M16 10l-2 2 2 2")
    val PanelExpand = stroke("panelExpand", 1.7f, rect(3.5f, 4f, 17f, 16f, 2f), "M9.5 4v16M14 10l2 2-2 2")
    val Paste = stroke("paste", 1.7f, rect(8f, 3f, 8f, 4f, 1f), "M16 5h2a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2")
    val Copy = stroke("copy", 1.7f, rect(8f, 8f, 12f, 12f, 2f), "M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2")
    val Paperclip = stroke("paperclip", 1.7f, "M13.77 8.82L8.82 13.77A1.75 1.75 0 0 0 11.29 16.24L17.66 9.88A3.5 3.5 0 0 0 12.71 4.93L5.64 12A4.5 4.5 0 0 0 12 18.36L16.6 13.77")
    val File = stroke("file", 1.7f, "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5")
    val Share = stroke("share", 1.7f, "M4 12v7a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-7M16 6l-4-4-4 4M12 2v13")
    val Key = stroke("key", 1.7f, circle(8f, 15f, 4f), "M11 12l8-8M16 7l2 2M14 9l2 2")
    val Download = stroke("download", 1.7f, "M12 3v12M7 10l5 5 5-5", "M5 19h14")
    val Lock = stroke("lock", 1.8f, rect(5f, 11f, 14f, 10f, 2f), "M8 11V8a4 4 0 0 1 8 0v3")
    val SecurityKey = stroke("securityKey", 1.7f, rect(7f, 2f, 10f, 14f, 2f), "M10 16v6h4v-6M12 6v4")
    val Server = stroke("server", 1.7f, rect(3f, 4f, 18f, 7f, 2f), rect(3f, 13f, 18f, 7f, 2f), "M7 7.5h.01M7 16.5h.01")
    val Bell = stroke("bell", 1.7f, "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.9 1.9 0 0 0 3.4 0")
    val More = stroke("more", 1f, circle(5f, 12f, 1.6f), circle(12f, 12f, 1.6f), circle(19f, 12f, 1.6f), filled = true)
    val ToEnd = stroke("toEnd", 1.8f, "M12 4v12M6 10l6 6 6-6M5 20h14")

    // Agent glyphs
    val Claude = stroke("claude", 1.8f, "M12 3v18M3 12h18M5.6 5.6l12.8 12.8M18.4 5.6 5.6 18.4")
    val Codex = stroke("codex", 1.8f, "M12 3l8 9-8 9-8-9z")

    // Workspace symbols
    val SymChart = stroke("symChart", 1.8f, "M4 20V10M10 20V4M16 20v-7M22 20H2")
    val SymCode = stroke("symCode", 1.8f, "M8 6l-5 6 5 6M16 6l5 6-5 6")
    val SymLayers = stroke("symLayers", 1.8f, "M12 3l9 5-9 5-9-5z", "M3 13l9 5 9-5")
    val SymDatabase = stroke("symDb", 1.8f, "M5 6a7 3 0 1 0 14 0a7 3 0 1 0-14 0", "M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3")
    val SymGlobe = stroke("symGlobe", 1.8f, circle(12f, 12f, 9f), "M3 12h18M12 3c3 3.2 3 14.8 0 18M12 3c-3 3.2-3 14.8 0 18")
    val SymRocket = stroke("symRocket", 1.8f, "M5 15c-1.5 1.5-2 5-2 5s3.5-.5 5-2M9 15l-3-3c1.5-4.5 5.5-9 13-9 0 7.5-4.5 11.5-9 13z", circle(14.5f, 9.5f, 1.5f))
    val SymWrench = stroke("symWrench", 1.8f, "M14.5 6.5a4 4 0 0 0 5 5L11 20a2.1 2.1 0 0 1-3-3l8.5-8.5")

    val workspaceSymbols: Map<String, ImageVector> = linkedMapOf(
        "chart" to SymChart, "code" to SymCode, "layers" to SymLayers, "database" to SymDatabase,
        "globe" to SymGlobe, "rocket" to SymRocket, "wrench" to SymWrench,
    )

    fun agent(type: String?): ImageVector? = when (type?.lowercase()) {
        "claude" -> Claude
        "codex" -> Codex
        null, "" -> null
        else -> Claude
    }
}
