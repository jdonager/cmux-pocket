package app.cmuxpocket.ui

import androidx.compose.ui.geometry.Offset
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

@Serializable
data class TerminalDisplayPreference(
    val fontSizeSp: Float = 14.5f,
    val fitWidth: Boolean = false,
    val panX: Float = 0f,
    val panY: Float? = null
) {
    fun normalized(): TerminalDisplayPreference = copy(
        fontSizeSp = if (fontSizeSp.isFinite()) fontSizeSp.coerceIn(2f, 48f) else 14.5f,
        panX = if (panX.isFinite()) panX else 0f,
        panY = panY?.takeIf { it.isFinite() }
    )
}

private val displayJson = Json { ignoreUnknownKeys = true; encodeDefaults = true }

internal fun terminalDisplayKey(host: String, port: Int, surfaceId: String): String =
    displayJson.encodeToString(listOf(host, port.toString(), surfaceId))

internal fun decodeTerminalDisplay(raw: String?, defaultFontSizeSp: Float): TerminalDisplayPreference =
    try {
        if (raw == null) TerminalDisplayPreference(fontSizeSp = defaultFontSizeSp).normalized()
        else displayJson.decodeFromString<TerminalDisplayPreference>(raw).normalized()
    } catch (_: Exception) {
        TerminalDisplayPreference(fontSizeSp = defaultFontSizeSp).normalized()
    }

internal fun encodeTerminalDisplay(preference: TerminalDisplayPreference): String =
    displayJson.encodeToString(preference.normalized())

/** Cell sizing shared by drawing, selection, and viewport controls. */
internal fun terminalCellWidth(
    availableWidth: Float,
    sourceCols: Int,
    requestedFontSizePx: Float,
    referenceAdvance: Float,
    fitWidth: Boolean
): Float {
    val baseWidth = if (fitWidth) availableWidth / sourceCols.coerceAtLeast(1)
        else requestedFontSizePx * referenceAdvance.coerceAtLeast(1f) / 100f
    return baseWidth
}

internal fun transformTerminalDisplay(
    preference: TerminalDisplayPreference,
    geometry: TerminalGeometry,
    nextFontSizeSp: Float,
    scaleRatio: Float,
    centroid: Offset,
    pan: Offset,
    isZooming: Boolean
): TerminalDisplayPreference {
    val contentWidth = geometry.sourceCols * geometry.cellWidth * scaleRatio
    val contentHeight = geometry.rows * geometry.rowHeight * scaleRatio
    val desiredX = centroid.x - (centroid.x - geometry.originX) * scaleRatio + pan.x
    val desiredY = centroid.y - (centroid.y - geometry.originY) * scaleRatio + pan.y
    return preference.copy(
        fontSizeSp = nextFontSizeSp,
        fitWidth = preference.fitWidth && !isZooming,
        panX = terminalPanX(geometry.canvasWidth - 8f, contentWidth, desiredX - 4f),
        panY = terminalPanY(geometry.canvasHeight, contentHeight, desiredY - (geometry.canvasHeight - contentHeight))
    ).normalized()
}

internal fun terminalPanX(availableWidth: Float, contentWidth: Float, panX: Float): Float =
    panX.coerceIn(minOf(0f, availableWidth - contentWidth), 0f)

internal fun terminalPanY(canvasHeight: Float, contentHeight: Float, panY: Float): Float =
    panY.coerceIn(0f, (contentHeight - canvasHeight).coerceAtLeast(0f))

internal fun terminalFocusPanY(canvasHeight: Float, contentHeight: Float, rowHeight: Float, focusRow: Int): Float =
    terminalPanY(canvasHeight, contentHeight, contentHeight - (focusRow.coerceAtLeast(0) + 1) * rowHeight)
