package ch.inscribe.missionctrl.ui

import androidx.compose.ui.text.AnnotatedString
import org.junit.Assert.assertEquals
import org.junit.Test

class ImagePathsTest {
    @Test fun findsPathsAgentsPrint() {
        assertEquals(listOf("/tmp/shot.png"), ImagePaths.find("● Read(/tmp/shot.png)"))
        assertEquals(listOf("~/Pictures/a-b_c.JPG"), ImagePaths.find("Saved to ~/Pictures/a-b_c.JPG."))
        assertEquals(listOf("out/chart.webp", "./x@2x.gif"), ImagePaths.find("wrote out/chart.webp and ./x@2x.gif"))
        assertEquals(listOf("docs/a.jpeg"), ImagePaths.find("`docs/a.jpeg`"))
    }

    @Test fun skipsUrlsAndOtherFiles() {
        assertEquals(emptyList<String>(), ImagePaths.find("see https://example.com/logo.png"))
        assertEquals(emptyList<String>(), ImagePaths.find("file.pngx notes.md image.svg"))
        assertEquals(emptyList<String>(), ImagePaths.find("../up/a.png"))
        assertEquals(emptyList<String>(), ImagePaths.find("a.png.txt a.png-old a.png_bak /tmp/a~1.png dir.png/x.txt a%20b.png"))
        assertEquals(listOf("/tmp/end.png"), ImagePaths.find("saved /tmp/end.png."))
        assertEquals(emptyList<String>(), ImagePaths.find("a.png+backup a.png@2 a.png._bak").filter { it == "a.png" })
    }

    @Test fun pathsBrokenAcrossRows() {
        assertEquals(true, ImagePaths.brokenAcross("Read(/tmp/sh", "ot.png)"))
        assertEquals(false, ImagePaths.brokenAcross("Use the client/server", "model"))
        assertEquals(false, ImagePaths.brokenAcross("/tmp/a.png", "/tmp/b.png"))
        assertEquals(false, ImagePaths.brokenAcross("see /tmp/sh ", "ot.png"))
        // A word-wrap before a path is no broken path.
        assertEquals(false, ImagePaths.brokenAcross("the shot was saved to", "/tmp/shots/home.png and"))
        assertEquals(false, ImagePaths.brokenAcross("look at the", "logo.png file"))
        assertEquals(false, ImagePaths.brokenAcross("compare it with", "./out/b.png"))
        assertEquals(false, ImagePaths.brokenAcross("saved (see", "/tmp/a.png)"))
        assertEquals(false, ImagePaths.brokenAcross("Use the client/server", "/tmp/screenshots/home.png"))
        // The middle row of a path over three rows: the rows after it are given run together.
        assertEquals(true, ImagePaths.brokenAcross("● Read(/tmp/aaaa/", "bbbbbbbbbbbbbbbbbbbbbbbbbb/" + "c.png)"))
        // A file listing near the pane width is no broken path.
        assertEquals(false, ImagePaths.brokenAcross("/home/u/app/src/main/ui/DetailScreen.kt", "/home/u/app/src/main/ui/Images.kt"))
        assertEquals(false, ImagePaths.brokenAcross("  android/app/src/ui/DetailScreen.kt", "android/app/src/ui/Images.kt"))
        assertEquals(false, ImagePaths.brokenAcross("  android/app/src/ui/DetailScreen.kt", "android/app/res/logo.png"))
        assertEquals(true, ImagePaths.brokenAcross("Read(/tmp/shot.p", "ng)"))
    }

    @Test fun previewsEachPathOnlyAtTheLineThatNamesItFirst() {
        // Index 0 is the newest line (the history view lists them bottom up).
        val lines = listOf("done: /tmp/a.png", "Read(/tmp/b.png)", "Read(/tmp/a.png)").map { DisplayLine(AnnotatedString(it)) }
        assertEquals(mapOf(2 to listOf("/tmp/a.png"), 1 to listOf("/tmp/b.png")), ImagePaths.anchors(lines))
        // A relative name of the same file counts as the same image.
        val named = listOf("saved /work/c.png", "make ./c.png please").map { DisplayLine(AnnotatedString(it)) }
        assertEquals(mapOf(0 to listOf("/work/c.png")), ImagePaths.anchors(named, "/work"))
    }
}
