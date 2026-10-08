package app.cmuxpocket

import app.cmuxpocket.ui.terminalCellWidth
import app.cmuxpocket.ui.TerminalDisplayPreference
import app.cmuxpocket.ui.decodeTerminalDisplay
import app.cmuxpocket.ui.encodeTerminalDisplay
import app.cmuxpocket.ui.terminalDisplayKey
import app.cmuxpocket.ui.terminalFocusPanY
import app.cmuxpocket.ui.terminalPanX
import app.cmuxpocket.ui.terminalPanY
import app.cmuxpocket.ui.TerminalGeometry
import app.cmuxpocket.ui.transformTerminalDisplay
import androidx.compose.ui.geometry.Offset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.assertNotEquals
import org.junit.Test

class TerminalDisplayTest {
    @Test
    fun readableTextRespectsTheFontSizeForAWideDesktopTerminal() {
        val small = width(fontPx = 14f)
        val large = width(fontPx = 24f)

        assertTrue("Increasing the font size must increase drawn cell width", large > small)
        assertEquals(14f * 0.6f, small, 0.001f)
        assertEquals(24f * 0.6f, large, 0.001f)
    }

    @Test
    fun desktopColumnCountDoesNotShrinkReadableText() {
        assertEquals(width(columns = 80), width(columns = 240), 0.001f)
    }

    @Test
    fun fitWidthStillShowsEveryColumn() {
        assertEquals(360f / 160, width(fit = true), 0.001f)
    }

    @Test
    fun viewsAreSavedSeparatelyForEachTerminalAndHost() {
        val first = TerminalDisplayPreference(fontSizeSp = 22f, panX = -120f, panY = 60f)
        val second = TerminalDisplayPreference(fontSizeSp = 12f, fitWidth = true)
        val firstKey = terminalDisplayKey("demo.example", 443, "terminal-one")
        val secondKey = terminalDisplayKey("demo.example", 443, "terminal-two")
        val saved = mapOf(firstKey to encodeTerminalDisplay(first), secondKey to encodeTerminalDisplay(second))

        assertEquals(first, decodeTerminalDisplay(saved[firstKey], 14.5f))
        assertEquals(second, decodeTerminalDisplay(saved[secondKey], 14.5f))
        assertNotEquals(firstKey, terminalDisplayKey("other.example", 443, "terminal-one"))
        assertNotEquals(firstKey, terminalDisplayKey("demo.example", 8443, "terminal-one"))
    }

    @Test
    fun unreadableSavedPreferencesFallBackToTheDefault() {
        assertEquals(TerminalDisplayPreference(fontSizeSp = 18f), decodeTerminalDisplay("invalid", 18f))
        assertEquals(TerminalDisplayPreference(fontSizeSp = 18f), decodeTerminalDisplay(null, 18f))
        assertEquals(48f, decodeTerminalDisplay("{\"fontSizeSp\":500}", 18f).fontSizeSp)
    }

    @Test
    fun panningStaysWithinTheGridAfterZoomOrRotation() {
        assertEquals(-600f, terminalPanX(360f, 960f, -2000f))
        assertEquals(0f, terminalPanX(800f, 600f, -120f))
        assertEquals(400f, terminalPanY(600f, 1000f, 900f))
        assertEquals(0f, terminalPanY(1200f, 1000f, 400f))
    }

    @Test
    fun aCursorNearTheTopOfATallGridRemainsVisible() {
        // 60 Mac rows, but only 20 fit on the phone. An early shell prompt must not disappear.
        assertEquals(800f, terminalFocusPanY(400f, 1200f, 20f, 7))
        // Later output keeps the cursor on the bottom edge of the phone's viewport.
        assertEquals(200f, terminalFocusPanY(400f, 1200f, 20f, 49))
        assertEquals(0f, terminalFocusPanY(400f, 1200f, 20f, 59))
    }

    @Test
    fun pinchZoomKeepsTheCellUnderTheFingersInPlace() {
        val geometry = TerminalGeometry(360f, 500f, 160, 60, 8f, 20f, 16f, 4f, -700f, 20f)
        val fingers = Offset(180f, 250f)
        val next = transformTerminalDisplay(
            TerminalDisplayPreference(fontSizeSp = 20f), geometry, 40f, 2f, fingers, Offset.Zero, true
        )
        val zoomed = geometry.copy(
            cellWidth = 16f,
            rowHeight = 40f,
            originX = 4f + next.panX,
            originY = 500f - 60 * 40f + next.panY!!,
            fontSizePx = 40f
        )
        assertEquals(geometry.pointToCell(fingers), zoomed.pointToCell(fingers))
        assertEquals(40f, next.fontSizeSp)
    }

    @Test
    fun pinchZoomFromFitWidthSavesTheActualRenderedSize() {
        val geometry = TerminalGeometry(360f, 500f, 160, 60, 2.2f, 5f, 4f, 4f, 200f, 4f)
        val next = transformTerminalDisplay(
            TerminalDisplayPreference(fitWidth = true), geometry, 8f, 2f, Offset(180f, 300f), Offset.Zero, true
        )
        assertEquals(false, next.fitWidth)
        assertEquals(8f, next.fontSizeSp)
        assertEquals(next, decodeTerminalDisplay(encodeTerminalDisplay(next), 14.5f))
    }

    private fun width(
        fontPx: Float = 18f,
        columns: Int = 160,
        fit: Boolean = false
    ) = terminalCellWidth(360f, columns, fontPx, 60f, fit)
}
