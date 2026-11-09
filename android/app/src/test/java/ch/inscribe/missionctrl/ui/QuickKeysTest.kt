package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class QuickKeysTest {
    @Test fun everyKeyIsInExactlyOneGroupOfTheSheet() {
        val grouped = QuickKeys.groups.flatMap { it.second }
        assertEquals(grouped.distinct(), grouped)
        assertEquals(QuickKeys.all.map { it.id }.toSet(), grouped.toSet())
    }

    @Test fun cardOffersNoFilesOrCopying() {
        assertTrue("mic" in QuickKeys.availableStart)
        assertTrue("left" in QuickKeys.availableStart)
        assertFalse("file" in QuickKeys.availableStart)
        assertFalse("copy" in QuickKeys.availableStart)
    }

    @Test fun modifiersAndPageKeysAreOfferedEverywhere() {
        assertTrue("ctrl" in QuickKeys.availableStart && "ctrlalt" in QuickKeys.availableDetail)
        assertTrue(listOf("home", "end", "pgup", "pgdn", "del").all { it in QuickKeys.availableStart && it in QuickKeys.availableDetail })
        assertEquals(listOf("pageup"), QuickKeys.get("pgup")?.keys)
    }

    @Test fun typedKeysBecomeHerdrNames() {
        assertEquals("r", ModKeys.name("r"))
        assertEquals("R", ModKeys.name("R")) // herdr: Shift + r
        assertEquals("enter", ModKeys.name("\r"))
        assertEquals("backspace", ModKeys.name("\u007F"))
        assertEquals("left", ModKeys.name("\u001B[D"))
        assertEquals("space", ModKeys.name(" "))
        assertEquals("plus", ModKeys.name("+"))
        assertEquals(null, ModKeys.name(""))
        assertEquals("pageup", ModKeys.name("\u001B[5~"))
        assertEquals("home", ModKeys.name("\u001B[H"))
        assertEquals(null, ModKeys.name("\u001B[2~"))
        assertEquals("ctrl+home", ModKeys.chord("ctrl", "home"))
        assertEquals("ctrl+alt+left", ModKeys.chord("ctrl+alt", "left"))
        assertEquals("shift+tab", ModKeys.chord("shift", ModKeys.name("\t")!!))
    }
}
