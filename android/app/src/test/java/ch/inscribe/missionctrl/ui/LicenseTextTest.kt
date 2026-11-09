package ch.inscribe.missionctrl.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class LicenseTextTest {
    private val wrapped = "Redistribution and use in source and binary forms, with or without\n" +
        "modification, are permitted provided that the following conditions are\n" +
        "met:"

    @Test fun wrappedLinesOfAParagraphAreJoined() {
        assertEquals(
            "Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:",
            LicenseText.reflow(wrapped),
        )
    }

    @Test fun paragraphsListItemsAndShortLinesStayApart() {
        val text = "MIT License\n\nCopyright (c) 2016 A. Author with a fairly long name, and co\n" +
            "Copyright (c) 2017 Someone Else\n\n" +
            "1. Redistributions of source code must retain the above copyright notice,\n" +
            "   this list of conditions and the following disclaimer.\n" +
            "2. Redistributions in binary form must reproduce the above copyright\n" +
            "   notice.\n"
        assertEquals(
            "MIT License\n\nCopyright (c) 2016 A. Author with a fairly long name, and co\n" +
                "Copyright (c) 2017 Someone Else\n\n" +
                "1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.\n" +
                "2. Redistributions in binary form must reproduce the above copyright notice.\n",
            LicenseText.reflow(text),
        )
    }

    @Test fun commentPrefixesRulesAndHyphensAreKept() {
        val comment = "// Permission to use, copy, modify, and/or distribute this software for any\n// purpose."
        assertEquals(comment, LicenseText.reflow(comment))
        val rule = "SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007 and some more\n-----------"
        assertEquals(rule, LicenseText.reflow(rule))
        assertEquals(
            "a royalty-free, irrevocable copyright license to reproduce, prepare non-exclusive works",
            LicenseText.reflow("a royalty-free, irrevocable copyright license to reproduce, prepare non-\nexclusive works"),
        )
    }
}
