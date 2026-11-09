package ch.inscribe.missionctrl.ui

import ch.inscribe.missionctrl.KeyVault
import org.junit.Assert.assertEquals
import org.junit.Test

class SshKeysTest {
    @Test
    fun nameBecomesTheServerComment() {
        assertEquals("Fold 8 (MissionCtrl)", KeyVault.comment("  Fold 8 "))
        val line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINgt missionctrl@SM-F971B"
        assertEquals("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINgt Laptop (MissionCtrl)", KeyVault.withComment(line, KeyVault.comment("Laptop")))
        assertEquals("ssh-ed25519 AAAA x (MissionCtrl)", KeyVault.withComment("ssh-ed25519 AAAA", KeyVault.comment("x")))
    }

    @Test
    fun exportFileNamesAreSafe() {
        assertEquals("id_ed25519_galaxy-z-fold8", exportFileName("Galaxy Z Fold8"))
        assertEquals("id_ed25519_r-ger-s-handy", exportFileName("Röger's Handy"))
        assertEquals("id_ed25519_missionctrl", exportFileName("✓"))
    }

    @Test
    fun hostKeyShowsItsFingerprint() {
        // The Pixel's app key; ssh-keygen -l gives this fingerprint (android-geraet.md).
        val line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHVTWxrfJjlTt/8KOq8CqB//etk9q628iD2zvrX+TwQ4 MissionCtrlPixel10 (MissionCtrl)"
        assertEquals("SHA256:Mx8MCQMH1VHk10a7l0VQO5qvgNrXEfpAftVGgdWBWzo", sshFingerprint(line))
        assertEquals(null, sshFingerprint("kein Schlüssel"))
    }

    @Test
    fun packageNamesBreakAtSeparators() {
        assertEquals("a-\u200Bb:\u200Bc.\u200Bd", breakable("a-b:c.d"))
    }
}
