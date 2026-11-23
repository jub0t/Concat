// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

package app.concat.editor.spike.ui.theme

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.concat.editor.spike.R
import app.concat.editor.spike.api.ClipKind

/*
 * Concat's identity on the phone: the desktop's dark theme
 * (src/crates/concat/ui/theme/dark.slint) as Compose tokens. One accent,
 * lime, for every primary action, the selection and the playhead.
 */

/** Surfaces, text, accent and status: dark.slint's values. */
object ConcatColors {
    val page = Color(0xFF0E0E10)
    val launchPanel = Color(0xFF161619)
    val launchRaised = Color(0xFF1E1E22)
    val launchField = Color(0xFF26262B)
    val fieldActive = Color(0xFF313138)
    val panel = Color(0xFF212123)
    val raised = Color(0xFF2B2B2E)
    val well = Color(0xFF19191B)
    val line = Color(0x1AFFFFFF)
    val lineStrong = Color(0x33FFFFFF)

    val fg = Color(0xFFF5F5F7)
    val fgMuted = Color(0xFFA1A1A6)
    val fgDim = Color(0xFF6E6E73)
    val fgHot = Color(0xFFFFFFFF)

    val accent = Color(0xFFCBF53F)
    val accentHover = Color(0xFFD9FF63)
    val accentSoft = Color(0x33CBF53F)
    /** Ink on the accent: text and icons on a lime button. */
    val onAccent = Color(0xFF15220A)

    val green = Color(0xFF30D158)
    val warn = Color(0xFFFFB340)
    val danger = Color(0xFFFF453A)
    val meterLow = Color(0xFF34C759)
    val meterMid = Color(0xFFFFD60A)
    val meterHigh = Color(0xFFFF453A)
}

/** A clip kind's colour and the darker well its body sits in. */
data class KindColor(val color: Color, val well: Color)

object KindColors {
    val video = KindColor(Color(0xFF3373D1), Color(0xFF142238))
    val audio = KindColor(Color(0xFF34C46F), Color(0xFF12301C))
    val image = KindColor(Color(0xFF9F63D6), Color(0xFF2A1E34))
    val text = KindColor(Color(0xFFCF5A99), Color(0xFF321C29))
    val filter = KindColor(Color(0xFF6D63E8), Color(0xFF23204A))
    val effect = KindColor(Color(0xFFF2674F), Color(0xFF3D1913))
    val adjustment = KindColor(Color(0xFFD153E0), Color(0xFF36173B))
    val transition = KindColor(Color(0xFF8FC23A), Color(0xFF24300F))
    val shape = KindColor(Color(0xFFD4A24B), Color(0xFF3A2D14))

    fun of(kind: ClipKind): KindColor = when (kind) {
        ClipKind.Video -> video
        ClipKind.Audio -> audio
        ClipKind.Image -> image
        ClipKind.Text -> text
        ClipKind.Layer -> adjustment
        ClipKind.SoundLayer -> filter
        ClipKind.Transition -> transition
        ClipKind.Other -> shape
    }
}

/** The bundled UI face: Inter, as the desktop ships it. */
val Inter = FontFamily(
    Font(R.font.inter_regular, FontWeight.Normal),
    Font(R.font.inter_regular, FontWeight.Medium),
    Font(R.font.inter_semibold, FontWeight.SemiBold),
    Font(R.font.inter_bold, FontWeight.Bold),
)

/** The type scale. Timecode uses tabular figures so it does not jitter. */
object ConcatType {
    val display = TextStyle(fontFamily = Inter, fontWeight = FontWeight.Bold, fontSize = 28.sp, letterSpacing = (-0.4).sp)
    val title = TextStyle(fontFamily = Inter, fontWeight = FontWeight.SemiBold, fontSize = 17.sp)
    val heading = TextStyle(fontFamily = Inter, fontWeight = FontWeight.SemiBold, fontSize = 15.sp)
    val body = TextStyle(fontFamily = Inter, fontWeight = FontWeight.Normal, fontSize = 14.sp)
    val label = TextStyle(fontFamily = Inter, fontWeight = FontWeight.Medium, fontSize = 12.sp)
    val caption = TextStyle(fontFamily = Inter, fontWeight = FontWeight.Medium, fontSize = 11.sp)
    val badge = TextStyle(fontFamily = Inter, fontWeight = FontWeight.SemiBold, fontSize = 10.sp)
    val timecode = TextStyle(
        fontFamily = Inter,
        fontWeight = FontWeight.Medium,
        fontSize = 12.sp,
        fontFeatureSettings = "tnum",
    )
}

/** Spacing on a 4dp grid. */
object Spacing {
    val xxs = 2.dp
    val xs = 4.dp
    val sm = 8.dp
    val md = 12.dp
    val lg = 16.dp
    val xl = 24.dp
    val xxl = 32.dp
}

/** Corner radii. */
object Radii {
    val xs = 4.dp
    val sm = 6.dp
    val md = 10.dp
    val lg = 14.dp
    val xl = 20.dp
}

/** The editor's fixed measurements (MOBILE_DESIGN.md). */
object Dimens {
    val topBar = 48.dp
    val transport = 44.dp
    val toolbar = 72.dp
    val mainTrack = 56.dp
    val overlayLane = 32.dp
    val emptyLane = 20.dp
    val ruler = 28.dp
    const val PREVIEW_SHARE = 0.42f
}

/** 200-250ms standard easing; springs for the playhead knob. */
object Motion {
    val standard = CubicBezierEasing(0.2f, 0f, 0f, 1f)
    const val SHORT = 200
    const val MEDIUM = 240
    fun <T> tweenShort(): FiniteAnimationSpec<T> = tween(SHORT, easing = standard)
    fun <T> tweenMedium(): FiniteAnimationSpec<T> = tween(MEDIUM, easing = standard)
    fun <T> knob(): FiniteAnimationSpec<T> = spring(dampingRatio = Spring.DampingRatioMediumBouncy, stiffness = Spring.StiffnessMedium)
}

private val typography = Typography().let { base ->
    Typography(
        displaySmall = ConcatType.display,
        headlineSmall = ConcatType.title.copy(fontSize = 20.sp),
        titleLarge = ConcatType.title,
        titleMedium = ConcatType.heading,
        titleSmall = ConcatType.label.copy(fontSize = 13.sp),
        bodyLarge = ConcatType.body.copy(fontSize = 15.sp),
        bodyMedium = ConcatType.body,
        bodySmall = ConcatType.label,
        labelLarge = ConcatType.label.copy(fontSize = 14.sp, fontWeight = FontWeight.SemiBold),
        labelMedium = ConcatType.label,
        labelSmall = ConcatType.caption,
        displayLarge = base.displayLarge.copy(fontFamily = Inter),
        displayMedium = base.displayMedium.copy(fontFamily = Inter),
        headlineLarge = base.headlineLarge.copy(fontFamily = Inter),
        headlineMedium = base.headlineMedium.copy(fontFamily = Inter),
    )
}

private val shapes = Shapes(
    extraSmall = RoundedCornerShape(Radii.xs),
    small = RoundedCornerShape(Radii.sm),
    medium = RoundedCornerShape(Radii.md),
    large = RoundedCornerShape(Radii.lg),
    extraLarge = RoundedCornerShape(Radii.xl),
)

@Composable
fun ConcatTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = darkColorScheme(
            primary = ConcatColors.accent,
            onPrimary = ConcatColors.onAccent,
            primaryContainer = ConcatColors.accentSoft,
            onPrimaryContainer = ConcatColors.accent,
            secondary = ConcatColors.accent,
            onSecondary = ConcatColors.onAccent,
            secondaryContainer = ConcatColors.raised,
            onSecondaryContainer = ConcatColors.fg,
            tertiary = ConcatColors.accent,
            background = ConcatColors.page,
            onBackground = ConcatColors.fg,
            surface = ConcatColors.launchPanel,
            onSurface = ConcatColors.fg,
            surfaceVariant = ConcatColors.raised,
            onSurfaceVariant = ConcatColors.fgMuted,
            surfaceContainerLowest = ConcatColors.page,
            surfaceContainerLow = ConcatColors.launchPanel,
            surfaceContainer = ConcatColors.panel,
            surfaceContainerHigh = ConcatColors.launchRaised,
            surfaceContainerHighest = ConcatColors.raised,
            outline = ConcatColors.lineStrong,
            outlineVariant = ConcatColors.line,
            error = ConcatColors.danger,
        ),
        typography = typography,
        shapes = shapes,
        content = content,
    )
}
