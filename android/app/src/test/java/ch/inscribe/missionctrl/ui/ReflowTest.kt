package ch.inscribe.missionctrl.ui

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.withStyle
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ReflowTest {
    private fun layout(lines: List<String>, cols: Int) = Reflow.layout(lines.map(::AnnotatedString), cols).map { it.text.text }

    @Test fun separatorsStayOnOneLine() {
        val sep = "─".repeat(80)
        val out = Reflow.layout(listOf(AnnotatedString(sep)), 40)
        assertTrue(out.single().noWrap)
        assertTrue(Reflow.isSeparator("  ──────────────── "))
        assertFalse(Reflow.isSeparator("── a b c d e f g h i"))
    }

    @Test fun joinsParagraphWrappedAtPaneWidth() {
        val a = "This is a long sentence the agent wrapped at the width of the laptop pane so"
        val b = "it continues here."
        assertEquals(listOf("$a $b"), layout(listOf(a, b), 40))
    }

    @Test fun keepsListItemsCodeAndShortLines() {
        val full = "x".repeat(70)
        assertEquals(2, layout(listOf(full, "- next item"), 40).size)
        assertEquals(2, layout(listOf("val a = 1; ".padEnd(70, 'y'), "b = 2"), 40).size)
        assertEquals(2, layout(listOf("short line", "another"), 40).size)
    }

    @Test fun nothingChangesWhenThePhoneIsWideEnough() {
        val a = "word ".repeat(15).trimEnd()
        assertEquals(listOf(a, "tail"), layout(listOf(a, "tail"), 200))
    }

    @Test fun hyphenatedWordIsJoinedWithoutSpace() {
        val a = "Wir brauchen einen sehr langen Satz mit einem getrennten Spiel-"
        assertEquals(listOf(a + "Merker hier."), layout(listOf(a, "Merker hier."), 30))
    }

    @Test fun pathBrokenByTheTerminalIsJoinedWhole() {
        val a = "see notes and then Read(/tmp/mct-r8/home/sh"
        assertEquals(listOf(a + "op/chart.png) done"), layout(listOf(a, "op/chart.png) done"), 30))
        val b = "a full line of prose that ends in words"
        assertEquals(listOf("$b and goes on"), layout(listOf(b, "and goes on"), 30))
        // Indented continuation under a list marker, as agents print tool calls.
        val c = "● Read(/tmp/mct-r8/home/shop/sh"
        assertEquals(listOf("● Read(/tmp/mct-r8/home/shop/shot.png)"), layout(listOf(c, "  ot.png)"), 20))
        // A path over four rows, wrapped by the terminal without indent under a marker.
        val rows = listOf(
            "● Read(/tmp/mct-r8/home/shop/screenshots-from-the-a",
            "gent-run/session-2026-11-19-evening/checkout-flow-d",
            "esktop-and-mobile/step-03-payment-form-filled-in.pn",
            "g)",
        )
        assertEquals(listOf(rows.joinToString("")), layout(rows, 30))
        // A slash in prose is no path to join.
        val d = "Use the client/server"
        assertEquals(listOf("$d model for this task"), layout(listOf(d, "model for this task"), 12))
    }

    @Test fun paragraphEndsAtAShortRow() {
        val a = "a full line of prose that ends in words"
        assertEquals(listOf("$a and stops.", "next line"), layout(listOf(a, "and stops.", "next line"), 30))
    }

    @Test fun hangUnderListText() {
        assertEquals(4, Reflow.hangOf("  - item"))
        assertEquals(3, Reflow.hangOf("1. item"))
    }

    @Test fun ruleKeepsItsLabel() {
        val rule = "─".repeat(80) + " Webshop ─"
        val out = Reflow.layout(listOf(AnnotatedString(rule)), 40).single()
        assertEquals(true, out.noWrap)
        assertEquals(40, out.text.text.length)
        assertEquals(true, out.text.text.endsWith(" Webshop ─"))
        // Fits already: unchanged.
        assertEquals("─".repeat(20), Reflow.shrinkRule(AnnotatedString("─".repeat(20)), 40).text)
    }

    @Test fun joiningSpaceKeepsBackground() {
        val grey = androidx.compose.ui.text.SpanStyle(background = androidx.compose.ui.graphics.Color.Gray)
        fun boxed(t: String) = androidx.compose.ui.text.buildAnnotatedString { withStyle(grey) { append(t) } }
        val a = boxed("> " + "wort ".repeat(15).trimEnd())
        val b = boxed("  weiter")
        val out = Reflow.layout(listOf(a, b, AnnotatedString("x".repeat(80))), 40)
        val joined = out.first().text
        val gap = joined.text.indexOf(" weiter")
        assertTrue(joined.spanStyles.any { it.item.background == androidx.compose.ui.graphics.Color.Gray && gap in it.start until it.end })
    }
}
