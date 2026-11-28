// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Authentic vertical Threads card and backdrop rendering for short-form video.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect,
    Stroke, Transform,
};

use super::{
    Align, Block, Error, Fonts, Rendered, TitleStyle, blur_within, encode_png, paint,
    push_rounded_rect,
};

/// Colour polarity for rendered Threads cards and backdrops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadsCardTheme {
    /// Dark frosted card over deep charcoal indigo backdrop.
    #[default]
    Dark,
    /// Bright card over soft warm pearl backdrop.
    Light,
}

/// Optional decoded avatar image (straight RGBA8, row-major).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarPixels {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Straight RGBA8 pixel buffer (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

/// Visual content for one rendered Threads card frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadsCardSpec {
    /// True for the root post card; false for a comment or custom note card.
    pub is_post: bool,
    /// Display name shown when no `@username` handle is available.
    pub author_name: String,
    /// Threads `@username` without the leading `@` (shown in bold like real Threads).
    pub author_handle: String,
    /// Relative post age shown beside the username (e.g. `"3d"`, `"5h"`).
    pub timestamp: String,
    /// True when the author account has a verified checkmark.
    pub verified: bool,
    /// Optional pill badge / fallback label (e.g. `"Comment 1"`).
    pub badge: String,
    /// Optional pre-formatted reply context line above the body copy.
    pub replying_to: String,
    /// Card body text (UTF-8, Vietnamese diacritics & color emoji supported).
    pub text: String,
    /// Formatted like count (empty omits the counter text).
    pub likes: String,
    /// Formatted reply count (empty omits the counter text).
    pub replies: String,
    /// Formatted repost count (empty omits the counter text).
    pub reposts: String,
    /// Optional decoded avatar photo; falls back to a monogram gradient circle.
    pub avatar: Option<AvatarPixels>,
    /// Optional decoded post/comment attached photo or video thumbnail.
    pub media: Option<AvatarPixels>,
    /// True when `media` is a video thumbnail (draws a subtle play indicator).
    pub media_is_video: bool,
    /// Card and backdrop colour theme polarity.
    pub theme: ThreadsCardTheme,
}

struct Palette {
    card_bg: Color,
    card_border: Color,
    shadow: Color,
    primary_text: Color,
    secondary_text: Color,
    icon_color: Color,
    badge_bg: Color,
    badge_text: Color,
    verified_bg: Color,
    avatar_ring: Color,
    plus_bg: Color,
    plus_fg: Color,
    media_border: Color,
}

fn palette(theme: ThreadsCardTheme) -> Palette {
    match theme {
        ThreadsCardTheme::Dark => Palette {
            card_bg: Color::from_rgba8(24, 24, 24, 250),
            card_border: Color::from_rgba8(255, 255, 255, 34),
            shadow: Color::from_rgba8(0, 0, 0, 150),
            primary_text: Color::from_rgba8(243, 245, 247, 255),
            secondary_text: Color::from_rgba8(136, 136, 136, 255),
            icon_color: Color::from_rgba8(190, 194, 200, 255),
            badge_bg: Color::from_rgba8(255, 255, 255, 16),
            badge_text: Color::from_rgba8(170, 174, 184, 255),
            verified_bg: Color::from_rgba8(0, 149, 246, 255),
            avatar_ring: Color::from_rgba8(255, 255, 255, 32),
            plus_bg: Color::from_rgba8(243, 245, 247, 255),
            plus_fg: Color::from_rgba8(16, 16, 16, 255),
            media_border: Color::from_rgba8(255, 255, 255, 28),
        },
        ThreadsCardTheme::Light => Palette {
            card_bg: Color::from_rgba8(255, 255, 255, 255),
            card_border: Color::from_rgba8(0, 0, 0, 26),
            shadow: Color::from_rgba8(15, 23, 42, 42),
            primary_text: Color::from_rgba8(15, 15, 15, 255),
            secondary_text: Color::from_rgba8(148, 152, 160, 255),
            icon_color: Color::from_rgba8(66, 66, 66, 255),
            badge_bg: Color::from_rgba8(15, 23, 42, 10),
            badge_text: Color::from_rgba8(110, 114, 122, 255),
            verified_bg: Color::from_rgba8(0, 149, 246, 255),
            avatar_ring: Color::from_rgba8(0, 0, 0, 22),
            plus_bg: Color::from_rgba8(16, 16, 16, 255),
            plus_fg: Color::from_rgba8(255, 255, 255, 255),
            media_border: Color::from_rgba8(0, 0, 0, 20),
        },
    }
}

/// Renders an opaque vertical aesthetic gradient backdrop for `T1` matching `theme`.
pub fn render_threads_backdrop(
    theme: ThreadsCardTheme,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Error> {
    let w = width.max(2);
    let h = height.max(2);
    let mut pixmap = Pixmap::new(w, h).ok_or(Error::Canvas(w, h))?;
    let wf = w as f32;
    let hf = h as f32;

    const BAYER4: [f32; 16] = [
        0.0 / 16.0,
        8.0 / 16.0,
        2.0 / 16.0,
        10.0 / 16.0,
        12.0 / 16.0,
        4.0 / 16.0,
        14.0 / 16.0,
        6.0 / 16.0,
        3.0 / 16.0,
        11.0 / 16.0,
        1.0 / 16.0,
        9.0 / 16.0,
        15.0 / 16.0,
        7.0 / 16.0,
        13.0 / 16.0,
        5.0 / 16.0,
    ];

    let data = pixmap.data_mut();
    for y in 0..h {
        let ny = y as f32 / (hf - 1.0);
        let bayer_row = ((y & 3) * 4) as usize;
        for x in 0..w {
            let nx = x as f32 / (wf - 1.0);
            let diag = 0.55 * ny + 0.45 * nx;

            // Two subtle atmospheric radial ambient glows on opposite corners.
            let d1x = (nx - 0.14) * 1.15;
            let d1y = (ny - 0.16) * 0.90;
            let glow1 = (1.0 - (d1x * d1x + d1y * d1y).sqrt() * 1.25).clamp(0.0, 1.0);
            let glow1 = glow1 * glow1;

            let d2x = (nx - 0.86) * 1.15;
            let d2y = (ny - 0.82) * 0.90;
            let glow2 = (1.0 - (d2x * d2x + d2y * d2y).sqrt() * 1.25).clamp(0.0, 1.0);
            let glow2 = glow2 * glow2;

            let (mut r, mut g, mut b) = match theme {
                ThreadsCardTheme::Dark => {
                    let base_r = 10.0 + 9.0 * (1.0 - diag);
                    let base_g = 12.0 + 9.0 * (1.0 - diag);
                    let base_b = 18.0 + 14.0 * (1.0 - diag);
                    (
                        base_r + 36.0 * glow1 + 48.0 * glow2,
                        base_g + 46.0 * glow1 + 18.0 * glow2,
                        base_b + 86.0 * glow1 + 52.0 * glow2,
                    )
                }
                ThreadsCardTheme::Light => {
                    let base_r = 247.0 - 11.0 * diag;
                    let base_g = 248.0 - 10.0 * diag;
                    let base_b = 252.0 - 6.0 * diag;
                    (
                        base_r - 22.0 * glow1 + 7.0 * glow2,
                        base_g - 10.0 * glow1 - 14.0 * glow2,
                        base_b + 3.0 * glow1 - 3.0 * glow2,
                    )
                }
            };

            let dither = BAYER4[bayer_row + (x as usize & 3)] - 0.46875;
            r = (r + dither).clamp(0.0, 255.0);
            g = (g + dither).clamp(0.0, 255.0);
            b = (b + dither).clamp(0.0, 255.0);

            let idx = ((y * w + x) * 4) as usize;
            data[idx] = r.round() as u8;
            data[idx + 1] = g.round() as u8;
            data[idx + 2] = b.round() as u8;
            data[idx + 3] = 255;
        }
    }

    encode_png(&pixmap)
}

/// Renders a crisp transparent-canvas authentic Threads post card centered
/// inside `width × height`, including optional embedded post/comment media.
pub fn render_threads_card(
    fonts: &Fonts,
    spec: &ThreadsCardSpec,
    width: u32,
    height: u32,
) -> Result<Rendered, Error> {
    let w = width.max(2);
    let h = height.max(2);
    let wf = w as f32;
    let hf = h as f32;
    let pal = palette(spec.theme);

    let portrait = h >= w;
    let card_w = if portrait {
        (wf * 0.90).clamp(280.0, (wf - 28.0).max(280.0))
    } else {
        (hf * 0.96).min(wf * 0.78).max(280.0)
    };
    let pad_x = (card_w * 0.048).round().max(16.0);
    let pad_y = (card_w * 0.042).round().max(16.0);
    let radius = (card_w * 0.032).clamp(14.0, 32.0);
    let inner_w = (card_w - 2.0 * pad_x).max(160.0);

    let avatar_d = (card_w * 0.082).clamp(34.0, 84.0).round();
    let header_gap = (card_w * 0.024).round().max(10.0);
    let dots_w = (card_w * 0.042).clamp(18.0, 40.0).round();
    let header_text_w = (inner_w - avatar_d - header_gap - dots_w - 14.0).max(120.0);

    // Authentic Threads header shows bold username (without `@`), falling back to display name.
    let primary_handle = {
        let h_clean = spec.author_handle.trim().trim_start_matches('@');
        if !h_clean.is_empty() {
            h_clean
        } else if !spec.author_name.trim().is_empty() {
            spec.author_name.trim()
        } else {
            "threads"
        }
    };

    let name_em = (card_w * 0.036).clamp(14.0, 34.0);
    let verified_size = if spec.verified {
        (name_em * 0.84).round().max(13.0)
    } else {
        0.0
    };
    let verified_gap = if spec.verified {
        (name_em * 0.24).round().max(4.0)
    } else {
        0.0
    };

    // Subtle relative time (`3d`) or comment badge beside username on the same single line.
    let meta_inline = if !spec.timestamp.trim().is_empty() {
        spec.timestamp.trim().to_owned()
    } else if !spec.is_post && !spec.badge.trim().is_empty() {
        spec.badge.trim().to_owned()
    } else {
        String::new()
    };
    let time_em = (card_w * 0.032).clamp(12.0, 30.0);
    let time_block = if meta_inline.is_empty() {
        None
    } else {
        Some(render_text_snippet(
            fonts,
            &meta_inline,
            400.0,
            pal.secondary_text,
            time_em,
            1.1,
            header_text_w * 0.45,
        )?)
    };
    let time_gap = if time_block.is_some() {
        (name_em * 0.42).round().max(8.0)
    } else {
        0.0
    };
    let time_w = time_block.as_ref().map_or(0.0, |b| b.width() as f32 + time_gap);
    let name_max_w = (header_text_w - verified_size - verified_gap - time_w).max(80.0);
    let name_block = render_text_snippet(
        fonts,
        primary_handle,
        700.0,
        pal.primary_text,
        name_em,
        1.15,
        name_max_w,
    )?;

    let header_h = avatar_d.max(name_block.height() as f32);

    let reply_em = (card_w * 0.028).clamp(11.0, 26.0);
    let reply_block = if spec.replying_to.trim().is_empty() {
        None
    } else {
        Some(render_text_snippet(
            fonts,
            spec.replying_to.trim(),
            400.0,
            pal.badge_text,
            reply_em,
            1.15,
            inner_w,
        )?)
    };
    let reply_h = reply_block.as_ref().map_or(0.0, |b| b.height() as f32);
    let reply_gap = if reply_block.is_some() {
        (pad_y * 0.34).round().max(8.0)
    } else {
        0.0
    };

    // Footer height & count blocks (no heavy divider line — clean Threads spacing).
    let footer_top_gap = (pad_y * 0.56).round().max(14.0);
    let icon_size = (card_w * 0.036).clamp(16.0, 34.0);
    let stat_em = (card_w * 0.030).clamp(12.0, 28.0);
    let likes_block = render_optional_stat(fonts, &spec.likes, pal.icon_color, stat_em)?;
    let replies_block = render_optional_stat(fonts, &spec.replies, pal.icon_color, stat_em)?;
    let reposts_block = render_optional_stat(fonts, &spec.reposts, pal.icon_color, stat_em)?;
    let footer_row_h = icon_size.max(stat_em * 1.25);
    let footer_h = footer_top_gap + footer_row_h;

    let valid_media = spec.media.as_ref().filter(|m| {
        m.width > 0 && m.height > 0 && m.rgba.len() >= (m.width * m.height * 4) as usize
    });
    let media_gap = if valid_media.is_some() {
        (pad_y * 0.52).round().max(12.0)
    } else {
        0.0
    };

    let body_top_gap = (pad_y * 0.50).round().max(12.0);
    let has_text = !spec.text.trim().is_empty();
    let max_card_h = if portrait { hf * 0.72 } else { hf * 0.84 };

    // Calculate embedded media display height while preserving aspect ratio cleanly.
    let media_h = if let Some(media) = valid_media {
        let aspect = (media.width as f32 / media.height as f32).clamp(0.78, 1.85);
        let natural_h = inner_w / aspect;
        let max_media_h = if portrait {
            (hf * 0.38).min(card_w * 0.95)
        } else {
            hf * 0.45
        };
        natural_h.min(max_media_h).round().max(100.0)
    } else {
        0.0
    };

    let fixed_h = 2.0 * pad_y
        + header_h
        + reply_gap
        + reply_h
        + if has_text { body_top_gap } else { 0.0 }
        + media_gap
        + media_h
        + footer_h;
    let max_body_h = (max_card_h - fixed_h).max(card_w * 0.14);

    let body_block = if has_text {
        Some(fit_body_text(
            fonts,
            &spec.text,
            pal.primary_text,
            card_w,
            inner_w,
            max_body_h,
        )?)
    } else {
        None
    };

    let body_h = body_block.as_ref().map_or(0.0, |b| b.height() as f32);
    let card_h = (fixed_h + body_h).min(hf - 24.0);
    let card_x = ((wf - card_w) * 0.5).round();
    let card_y = if portrait {
        ((hf * 0.44) - card_h * 0.5)
            .clamp(hf * 0.055, (hf - card_h - hf * 0.16).max(hf * 0.055))
            .round()
    } else {
        ((hf - card_h) * 0.5).round()
    };

    let mut canvas = Pixmap::new(w, h).ok_or(Error::Canvas(w, h))?;
    let card_rect =
        Rect::from_xywh(card_x, card_y, card_w, card_h).ok_or(Error::Canvas(w, h))?;

    draw_card_shadow(&mut canvas, card_rect, radius, pal.shadow)?;
    draw_card_surface(&mut canvas, card_rect, radius, pal.card_bg, pal.card_border)?;

    let inner_x = card_x + pad_x;
    let header_y = card_y + pad_y;

    // Avatar + signature Threads `+` follow badge on bottom-right corner.
    let av_y = header_y + ((header_h - avatar_d) * 0.5).round();
    draw_avatar(
        &mut canvas,
        fonts,
        spec,
        &pal,
        inner_x,
        av_y,
        avatar_d,
    )?;

    // Single-row header: bold username + optional verified badge + muted relative age.
    let mut hx = inner_x + avatar_d + header_gap;
    let name_y = header_y + ((header_h - name_block.height() as f32) * 0.5).round();
    name_block.paint_onto(&mut canvas, hx, name_y);
    hx += (name_block.width() as f32).min(name_max_w);
    if spec.verified {
        hx += verified_gap;
        let vy = header_y + ((header_h - verified_size) * 0.5).round();
        draw_verified_badge(&mut canvas, hx, vy, verified_size, pal.verified_bg)?;
        hx += verified_size;
    }
    if let Some(tb) = &time_block {
        hx += time_gap;
        let ty = header_y + ((header_h - tb.height() as f32) * 0.5).round();
        tb.paint_onto(&mut canvas, hx, ty);
    }

    // Subtle `···` overflow menu icon anchored right in the header (like real Threads).
    let dots_x = inner_x + inner_w - dots_w;
    let dots_cy = header_y + header_h * 0.5;
    draw_more_dots(&mut canvas, dots_x, dots_cy, dots_w, pal.secondary_text);

    // Optional reply context line for comments.
    let mut cursor_y = header_y + header_h;
    if let Some(rb) = &reply_block {
        cursor_y += reply_gap;
        rb.paint_onto(&mut canvas, inner_x, cursor_y);
        cursor_y += rb.height() as f32;
    }

    // Card body text.
    if let Some(bb) = &body_block {
        cursor_y += body_top_gap;
        bb.paint_onto(&mut canvas, inner_x, cursor_y);
        cursor_y += bb.height() as f32;
    }

    // Embedded post/comment photo or video thumbnail with rounded corners.
    if let Some(media) = valid_media {
        cursor_y += media_gap;
        let media_radius = (card_w * 0.024).clamp(10.0, 22.0);
        if let Some(mrect) = Rect::from_xywh(inner_x, cursor_y, inner_w, media_h) {
            draw_rounded_media(
                &mut canvas,
                media,
                mrect,
                media_radius,
                pal.media_border,
                spec.media_is_video,
            )?;
        }
        cursor_y += media_h;
    }

    // Authentic Threads engagement icons row (Heart, Comment, Repost, Share, Bookmark).
    cursor_y += footer_top_gap;
    draw_engagement_row(
        &mut canvas,
        &pal,
        inner_x,
        cursor_y,
        inner_w,
        footer_row_h,
        icon_size,
        [&likes_block, &replies_block, &reposts_block],
    )?;

    let png = encode_png(&canvas)?;
    let block_dx = ((card_x + card_w * 0.5) - wf * 0.5).round() as i32;
    let block_dy = ((card_y + card_h * 0.5) - hf * 0.5).round() as i32;
    Ok(Rendered {
        png,
        width: w,
        height: h,
        overscan: 1.0,
        block_width: card_w.round() as u32,
        block_height: card_h.round() as u32,
        block_dx,
        block_dy,
        words: Vec::new(),
    })
}

struct TextSnippet {
    pixmap: Pixmap,
    block: Block,
}

impl TextSnippet {
    fn width(&self) -> u32 {
        self.block.0
    }

    fn height(&self) -> u32 {
        self.block.1
    }

    fn paint_onto(&self, target: &mut Pixmap, dest_x: f32, dest_y: f32) {
        if self.block.0 == 0 || self.block.1 == 0 {
            return;
        }
        let cw = self.pixmap.width() as f32;
        let ch = self.pixmap.height() as f32;
        let block_left = cw * 0.5 + self.block.2 as f32 - self.block.0 as f32 * 0.5;
        let block_top = ch * 0.5 + self.block.3 as f32 - self.block.1 as f32 * 0.5;
        target.draw_pixmap(
            0,
            0,
            self.pixmap.as_ref(),
            &PixmapPaint::default(),
            Transform::from_translate(dest_x - block_left, dest_y - block_top),
            None,
        );
    }
}

fn hex_rgba(color: Color) -> String {
    let u = color.to_color_u8();
    format!("#{:02x}{:02x}{:02x}{:02x}", u.red(), u.green(), u.blue(), u.alpha())
}

fn render_text_snippet(
    fonts: &Fonts,
    text: &str,
    weight: f64,
    color: Color,
    em_px: f32,
    line_height: f64,
    max_width_px: f32,
) -> Result<TextSnippet, Error> {
    let content = text.replace('\t', "  ");
    if content.trim().is_empty() {
        let pm = Pixmap::new(2, 2).ok_or(Error::Canvas(2, 2))?;
        return Ok(TextSnippet {
            pixmap: pm,
            block: (0, 0, 0, 0),
        });
    }
    let max_w = max_width_px.max(32.0);
    // Give generous symmetric virtual canvas so left-aligned block starts at centre
    // and never needs extra overscan scaling for typical text/emoji ink.
    let vw = ((max_w * 2.4).ceil() as u32).clamp(320, 4096);
    let vh = 2200u32;
    let style = TitleStyle {
        content,
        font_family: "Hanken Grotesk".to_owned(),
        font_size: (em_px as f64 / vh as f64).clamp(0.005, 0.5),
        font_weight: weight,
        italic: false,
        underline: false,
        strikethrough: false,
        color: hex_rgba(color),
        align: Align::Left,
        stroke_width: 0.0,
        stroke_color: "#00000000".to_owned(),
        shadow: false,
        background: String::new(),
        background_radius: 0.0,
        background_padding_x: 0.0,
        background_padding_y: 0.0,
        line_height,
        tracking: 0.0,
        max_width: (max_w as f64 / vw as f64).clamp(0.05, 0.9),
        max_height: 0.0,
        runs: Vec::new(),
    };
    let (pixmap, block, _) = paint(fonts, &style, vw, vh)?;
    Ok(TextSnippet { pixmap, block })
}

fn render_optional_stat(
    fonts: &Fonts,
    value: &str,
    color: Color,
    em_px: f32,
) -> Result<Option<TextSnippet>, Error> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(render_text_snippet(
        fonts,
        trimmed,
        500.0,
        color,
        em_px,
        1.1,
        220.0,
    )?))
}

fn fit_body_text(
    fonts: &Fonts,
    raw_text: &str,
    color: Color,
    card_w: f32,
    inner_w: f32,
    max_body_h: f32,
) -> Result<TextSnippet, Error> {
    let normalized = raw_text.trim();
    let display = if normalized.is_empty() { " " } else { normalized };
    let base_em = (card_w * 0.044).clamp(16.0, 44.0);
    let mut best: Option<TextSnippet> = None;

    for factor in [1.0f32, 0.88, 0.78, 0.69, 0.61] {
        let em = (base_em * factor).max(13.0);
        let snippet = render_text_snippet(fonts, display, 400.0, color, em, 1.36, inner_w)?;
        let fits = (snippet.height() as f32) <= max_body_h;
        best = Some(snippet);
        if fits {
            return Ok(best.expect("assigned above"));
        }
    }

    // Even at minimum readable scale it exceeds the safe area: truncate cleanly at
    // a word boundary and append an ellipsis so cards never spill off-screen.
    let min_em = (base_em * 0.61).max(13.0);
    let chars: Vec<char> = display.chars().collect();
    let mut lo = 1usize;
    let mut hi = chars.len();
    let mut best_fit = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let candidate = truncate_with_ellipsis(&chars, mid);
        let snippet = render_text_snippet(fonts, &candidate, 400.0, color, min_em, 1.36, inner_w)?;
        if (snippet.height() as f32) <= max_body_h {
            best_fit = Some(snippet);
            lo = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            hi = mid - 1;
        }
    }

    Ok(best_fit.or(best).expect("always populated"))
}

fn truncate_with_ellipsis(chars: &[char], limit: usize) -> String {
    if limit >= chars.len() {
        return chars.iter().collect();
    }
    let slice: String = chars[..limit].iter().collect();
    let trimmed = match slice.rfind(char::is_whitespace) {
        Some(pos) if pos > slice.len() / 2 => slice[..pos].trim_end(),
        _ => slice.trim_end(),
    };
    format!("{trimmed}…")
}

fn draw_card_shadow(
    canvas: &mut Pixmap,
    rect: Rect,
    radius: f32,
    shadow_color: Color,
) -> Result<(), Error> {
    let spread = (rect.width() * 0.032).round().max(6.0);
    let offset_y = (rect.width() * 0.018).round().max(4.0);
    let mut shadow_pm = Pixmap::new(canvas.width(), canvas.height())
        .ok_or_else(|| Error::Canvas(canvas.width(), canvas.height()))?;

    let mut builder = PathBuilder::new();
    let sr = Rect::from_xywh(rect.left(), rect.top() + offset_y, rect.width(), rect.height())
        .ok_or_else(|| Error::Canvas(canvas.width(), canvas.height()))?;
    push_rounded_rect(&mut builder, sr, radius);
    if let Some(path) = builder.finish() {
        let mut paint = Paint::default();
        paint.set_color(shadow_color);
        paint.anti_alias = true;
        shadow_pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    let pad = (spread * 2.4).ceil() as i32;
    let x0 = (sr.left().floor() as i32 - pad).max(0) as usize;
    let y0 = (sr.top().floor() as i32 - pad).max(0) as usize;
    let x1 = (sr.right().ceil() as i32 + pad).min(canvas.width() as i32) as usize;
    let y1 = (sr.bottom().ceil() as i32 + pad).min(canvas.height() as i32) as usize;
    blur_within(&mut shadow_pm, spread as usize, (x0, y0, x1, y1));

    canvas.draw_pixmap(
        0,
        0,
        shadow_pm.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    Ok(())
}

fn draw_card_surface(
    canvas: &mut Pixmap,
    rect: Rect,
    radius: f32,
    fill: Color,
    border: Color,
) -> Result<(), Error> {
    let mut builder = PathBuilder::new();
    push_rounded_rect(&mut builder, rect, radius);
    if let Some(path) = builder.finish() {
        let mut paint = Paint::default();
        paint.set_color(fill);
        paint.anti_alias = true;
        canvas.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);

        let mut stroke_paint = Paint::default();
        stroke_paint.set_color(border);
        stroke_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.5,
            ..Default::default()
        };
        canvas.stroke_path(
            &path,
            &stroke_paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }
    Ok(())
}

fn draw_avatar(
    canvas: &mut Pixmap,
    fonts: &Fonts,
    spec: &ThreadsCardSpec,
    pal: &Palette,
    x: f32,
    y: f32,
    diam: f32,
) -> Result<(), Error> {
    let d_u32 = diam.round().max(8.0) as u32;
    let mut circle_pm = Pixmap::new(d_u32, d_u32).ok_or(Error::Canvas(d_u32, d_u32))?;
    let center = d_u32 as f32 * 0.5;
    let radius = center - 0.5;

    let valid_photo = spec.avatar.as_ref().filter(|av| {
        av.width > 0 && av.height > 0 && av.rgba.len() >= (av.width * av.height * 4) as usize
    });

    if let Some(photo) = valid_photo {
        blit_circular_photo(&mut circle_pm, photo, center, radius);
    } else {
        draw_monogram_gradient(&mut circle_pm, spec, center, radius);
        let initial = monogram_char(&spec.author_name, &spec.author_handle);
        let em = d_u32 as f32 * 0.44;
        let snippet = render_text_snippet(
            fonts,
            &initial,
            700.0,
            Color::WHITE,
            em,
            1.0,
            d_u32 as f32,
        )?;
        let tx = (d_u32 as f32 - snippet.width() as f32) * 0.5;
        let ty = (d_u32 as f32 - snippet.height() as f32) * 0.5;
        snippet.paint_onto(&mut circle_pm, tx, ty);
    }

    canvas.draw_pixmap(
        0,
        0,
        circle_pm.as_ref(),
        &PixmapPaint::default(),
        Transform::from_translate(x, y),
        None,
    );

    if let Some(ring_path) = PathBuilder::from_circle(x + center, y + center, radius) {
        let mut p = Paint::default();
        p.set_color(pal.avatar_ring);
        p.anti_alias = true;
        let stroke = Stroke {
            width: 1.5,
            ..Default::default()
        };
        canvas.stroke_path(&ring_path, &p, &stroke, Transform::identity(), None);
    }

    // Signature Threads `+` follow badge tucked onto the bottom-right corner of the avatar.
    let plus_r = (diam * 0.21).clamp(7.0, 18.0);
    let plus_cx = x + diam - plus_r * 0.55;
    let plus_cy = y + diam - plus_r * 0.55;
    if let Some(outer) = PathBuilder::from_circle(plus_cx, plus_cy, plus_r + 2.0) {
        let mut p = Paint::default();
        p.set_color(pal.card_bg);
        p.anti_alias = true;
        canvas.fill_path(&outer, &p, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(inner) = PathBuilder::from_circle(plus_cx, plus_cy, plus_r) {
        let mut p = Paint::default();
        p.set_color(pal.plus_bg);
        p.anti_alias = true;
        canvas.fill_path(&inner, &p, FillRule::Winding, Transform::identity(), None);
    }
    let arm = plus_r * 0.48;
    let mut pb = PathBuilder::new();
    pb.move_to(plus_cx - arm, plus_cy);
    pb.line_to(plus_cx + arm, plus_cy);
    pb.move_to(plus_cx, plus_cy - arm);
    pb.line_to(plus_cx, plus_cy + arm);
    stroke_icon(canvas, pb.finish(), (plus_r * 0.26).max(1.8), pal.plus_fg);

    Ok(())
}

fn blit_circular_photo(target: &mut Pixmap, src: &AvatarPixels, center: f32, radius: f32) {
    let w = target.width();
    let h = target.height();
    let sw = src.width as f32;
    let sh = src.height as f32;
    let crop_side = sw.min(sh);
    let ox = (sw - crop_side) * 0.5;
    let oy = (sh - crop_side) * 0.5;
    let data = target.data_mut();

    for py in 0..h {
        let dy = (py as f32 + 0.5) - center;
        let ny = (py as f32 + 0.5) / h as f32;
        for px in 0..w {
            let dx = (px as f32 + 0.5) - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = (radius + 0.6 - dist).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let nx = (px as f32 + 0.5) / w as f32;
            let sx = (ox + nx * crop_side - 0.5).clamp(0.0, sw - 1.0);
            let sy = (oy + ny * crop_side - 0.5).clamp(0.0, sh - 1.0);
            let [r, g, b, a] = sample_bilinear(src, sx, sy);
            let alpha = (a / 255.0) * cov;
            let idx = ((py * w + px) * 4) as usize;
            data[idx] = ((r * alpha).round()).clamp(0.0, 255.0) as u8;
            data[idx + 1] = ((g * alpha).round()).clamp(0.0, 255.0) as u8;
            data[idx + 2] = ((b * alpha).round()).clamp(0.0, 255.0) as u8;
            data[idx + 3] = ((alpha * 255.0).round()).clamp(0.0, 255.0) as u8;
        }
    }
}

fn sample_bilinear(src: &AvatarPixels, sx: f32, sy: f32) -> [f32; 4] {
    let x0 = sx.floor() as u32;
    let y0 = sy.floor() as u32;
    let x1 = (x0 + 1).min(src.width - 1);
    let y1 = (y0 + 1).min(src.height - 1);
    let tx = sx - x0 as f32;
    let ty = sy - y0 as f32;

    let pixel = |x: u32, y: u32| -> [f32; 4] {
        let idx = ((y * src.width + x) * 4) as usize;
        [
            src.rgba[idx] as f32,
            src.rgba[idx + 1] as f32,
            src.rgba[idx + 2] as f32,
            src.rgba[idx + 3] as f32,
        ]
    };
    let p00 = pixel(x0, y0);
    let p10 = pixel(x1, y0);
    let p01 = pixel(x0, y1);
    let p11 = pixel(x1, y1);
    let mut out = [0.0; 4];
    for i in 0..4 {
        let top = p00[i] + (p10[i] - p00[i]) * tx;
        let bot = p01[i] + (p11[i] - p01[i]) * tx;
        out[i] = top + (bot - top) * ty;
    }
    out
}

fn draw_monogram_gradient(target: &mut Pixmap, spec: &ThreadsCardSpec, center: f32, radius: f32) {
    const GRADIENTS: [([u8; 3], [u8; 3]); 6] = [
        ([99, 102, 241], [168, 85, 247]),  // indigo -> violet
        ([236, 72, 153], [249, 115, 22]),  // rose -> tangerine
        ([14, 165, 233], [59, 130, 246]),  // sky -> azure
        ([16, 185, 129], [20, 184, 166]),  // emerald -> teal
        ([245, 158, 11], [239, 68, 68]),   // amber -> coral
        ([139, 92, 246], [236, 72, 153]),  // purple -> pink
    ];

    let mut hasher = DefaultHasher::new();
    let seed = if spec.author_handle.trim().is_empty() {
        spec.author_name.trim()
    } else {
        spec.author_handle.trim()
    };
    seed.hash(&mut hasher);
    let (c0, c1) = GRADIENTS[(hasher.finish() as usize) % GRADIENTS.len()];

    let w = target.width();
    let h = target.height();
    let data = target.data_mut();
    for py in 0..h {
        let dy = (py as f32 + 0.5) - center;
        let ny = py as f32 / (h as f32 - 1.0).max(1.0);
        for px in 0..w {
            let dx = (px as f32 + 0.5) - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = (radius + 0.6 - dist).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let t = (0.55 * (px as f32 / (w as f32 - 1.0).max(1.0)) + 0.45 * ny).clamp(0.0, 1.0);
            let r = c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * t;
            let g = c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * t;
            let b = c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * t;
            let idx = ((py * w + px) * 4) as usize;
            data[idx] = (r * cov).round() as u8;
            data[idx + 1] = (g * cov).round() as u8;
            data[idx + 2] = (b * cov).round() as u8;
            data[idx + 3] = (cov * 255.0).round() as u8;
        }
    }
}

fn monogram_char(author_name: &str, author_handle: &str) -> String {
    let source = if !author_name.trim().is_empty() {
        author_name.trim()
    } else {
        author_handle.trim().trim_start_matches('@')
    };
    source
        .chars()
        .find(|c| !c.is_whitespace())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "T".to_owned())
}

fn draw_verified_badge(
    canvas: &mut Pixmap,
    x: f32,
    y: f32,
    size: f32,
    fill: Color,
) -> Result<(), Error> {
    let r = size * 0.5;
    if let Some(circle) = PathBuilder::from_circle(x + r, y + r, r) {
        let mut p = Paint::default();
        p.set_color(fill);
        p.anti_alias = true;
        canvas.fill_path(&circle, &p, FillRule::Winding, Transform::identity(), None);
    }

    let mut cb = PathBuilder::new();
    cb.move_to(x + size * 0.28, y + size * 0.52);
    cb.line_to(x + size * 0.44, y + size * 0.67);
    cb.line_to(x + size * 0.74, y + size * 0.35);
    if let Some(check) = cb.finish() {
        let mut p = Paint::default();
        p.set_color(Color::WHITE);
        p.anti_alias = true;
        let stroke = Stroke {
            width: (size * 0.13).max(1.5),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        canvas.stroke_path(&check, &p, &stroke, Transform::identity(), None);
    }
    Ok(())
}

/// Draws the subtle horizontal `···` overflow menu indicator anchored on the right of the header.
fn draw_more_dots(canvas: &mut Pixmap, x: f32, cy: f32, width: f32, color: Color) {
    let r = (width * 0.088).clamp(2.0, 4.0);
    let step = width * 0.34;
    let start_x = x + (width - step * 2.0) * 0.5;
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    for i in 0..3 {
        if let Some(dot) = PathBuilder::from_circle(start_x + i as f32 * step, cy, r) {
            canvas.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

/// Renders an attached Threads post/comment photo (or video thumbnail) inside a
/// rounded rectangle with cover-crop bilinear sampling and a subtle hairline border.
fn draw_rounded_media(
    canvas: &mut Pixmap,
    src: &AvatarPixels,
    rect: Rect,
    radius: f32,
    border: Color,
    is_video: bool,
) -> Result<(), Error> {
    let w = rect.width().round().max(2.0) as u32;
    let h = rect.height().round().max(2.0) as u32;
    let mut pm = Pixmap::new(w, h).ok_or(Error::Canvas(w, h))?;

    let dw = w as f32;
    let dh = h as f32;
    let sw = src.width as f32;
    let sh = src.height as f32;
    let r = radius.min(dw * 0.5).min(dh * 0.5);

    // Cover-crop source rect so the image fills the rounded frame without distortion.
    let scale = (dw / sw).max(dh / sh);
    let crop_w = dw / scale;
    let crop_h = dh / scale;
    let ox = (sw - crop_w) * 0.5;
    let oy = (sh - crop_h) * 0.5;

    let data = pm.data_mut();
    for py in 0..h {
        let fy = py as f32 + 0.5;
        let sy = (oy + (fy / dh) * crop_h - 0.5).clamp(0.0, sh - 1.0);
        for px in 0..w {
            let fx = px as f32 + 0.5;
            let cov = rounded_rect_coverage(fx, fy, dw, dh, r);
            if cov <= 0.0 {
                continue;
            }
            let sx = (ox + (fx / dw) * crop_w - 0.5).clamp(0.0, sw - 1.0);
            let [cr, cg, cb, ca] = sample_bilinear(src, sx, sy);
            let alpha = (ca / 255.0) * cov;
            let idx = ((py * w + px) * 4) as usize;
            data[idx] = (cr * alpha).round().clamp(0.0, 255.0) as u8;
            data[idx + 1] = (cg * alpha).round().clamp(0.0, 255.0) as u8;
            data[idx + 2] = (cb * alpha).round().clamp(0.0, 255.0) as u8;
            data[idx + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    canvas.draw_pixmap(
        0,
        0,
        pm.as_ref(),
        &PixmapPaint::default(),
        Transform::from_translate(rect.left(), rect.top()),
        None,
    );

    let mut builder = PathBuilder::new();
    push_rounded_rect(&mut builder, rect, r);
    if let Some(path) = builder.finish() {
        let mut p = Paint::default();
        p.set_color(border);
        p.anti_alias = true;
        let stroke = Stroke {
            width: 1.5,
            ..Default::default()
        };
        canvas.stroke_path(&path, &p, &stroke, Transform::identity(), None);
    }

    if is_video {
        let cx = rect.left() + rect.width() * 0.5;
        let cy = rect.top() + rect.height() * 0.5;
        let pr = (rect.width().min(rect.height()) * 0.12).clamp(22.0, 48.0);
        if let Some(circle) = PathBuilder::from_circle(cx, cy, pr) {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(0, 0, 0, 140));
            p.anti_alias = true;
            canvas.fill_path(&circle, &p, FillRule::Winding, Transform::identity(), None);
        }
        let mut tri = PathBuilder::new();
        tri.move_to(cx - pr * 0.28, cy - pr * 0.42);
        tri.line_to(cx + pr * 0.48, cy);
        tri.line_to(cx - pr * 0.28, cy + pr * 0.42);
        tri.close();
        if let Some(tpath) = tri.finish() {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(255, 255, 255, 240));
            p.anti_alias = true;
            canvas.fill_path(&tpath, &p, FillRule::Winding, Transform::identity(), None);
        }
    }

    Ok(())
}

fn rounded_rect_coverage(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = if x < r {
        r
    } else if x > w - r {
        w - r
    } else {
        return 1.0;
    };
    let cy = if y < r {
        r
    } else if y > h - r {
        h - r
    } else {
        return 1.0;
    };
    let dx = x - cx;
    let dy = y - cy;
    let dist = (dx * dx + dy * dy).sqrt();
    (r + 0.6 - dist).clamp(0.0, 1.0)
}

fn draw_engagement_row(
    canvas: &mut Pixmap,
    pal: &Palette,
    x: f32,
    y: f32,
    inner_w: f32,
    row_h: f32,
    icon_size: f32,
    counts: [&Option<TextSnippet>; 3],
) -> Result<(), Error> {
    let gap_label = (icon_size * 0.34).round().max(5.0);
    let group_gap = (inner_w * 0.044).round().max(16.0);
    let icon_y = y + (row_h - icon_size) * 0.5;
    let stroke_w = (icon_size * 0.075).max(1.6);

    let mut cur_x = x;

    // 1. Heart + Likes
    draw_heart_icon(canvas, cur_x, icon_y, icon_size, stroke_w, pal.icon_color);
    cur_x += icon_size;
    if let Some(lbl) = counts[0] {
        cur_x += gap_label;
        let ly = y + (row_h - lbl.height() as f32) * 0.5;
        lbl.paint_onto(canvas, cur_x, ly);
        cur_x += lbl.width() as f32;
    }
    cur_x += group_gap;

    // 2. Comment bubble + Replies
    draw_comment_icon(canvas, cur_x, icon_y, icon_size, stroke_w, pal.icon_color);
    cur_x += icon_size;
    if let Some(lbl) = counts[1] {
        cur_x += gap_label;
        let ly = y + (row_h - lbl.height() as f32) * 0.5;
        lbl.paint_onto(canvas, cur_x, ly);
        cur_x += lbl.width() as f32;
    }
    cur_x += group_gap;

    // 3. Repost arrows + Reposts
    draw_repost_icon(canvas, cur_x, icon_y, icon_size, stroke_w, pal.icon_color);
    cur_x += icon_size;
    if let Some(lbl) = counts[2] {
        cur_x += gap_label;
        let ly = y + (row_h - lbl.height() as f32) * 0.5;
        lbl.paint_onto(canvas, cur_x, ly);
        cur_x += lbl.width() as f32;
    }
    cur_x += group_gap;

    // 4. Share paper-plane icon
    if cur_x + icon_size <= x + inner_w {
        draw_share_icon(canvas, cur_x, icon_y, icon_size, stroke_w, pal.icon_color);
        cur_x += icon_size + group_gap;
    }

    // 5. Bookmark icon (matches real Threads action row)
    if cur_x + icon_size <= x + inner_w {
        draw_bookmark_icon(canvas, cur_x, icon_y, icon_size, stroke_w, pal.icon_color);
    }

    Ok(())
}

fn draw_bookmark_icon(canvas: &mut Pixmap, x: f32, y: f32, s: f32, sw: f32, color: Color) {
    let p = |nx: f32, ny: f32| (x + nx * s, y + ny * s);
    let mut pb = PathBuilder::new();
    pb.move_to(p(0.24, 0.16).0, p(0.24, 0.16).1);
    pb.line_to(p(0.76, 0.16).0, p(0.76, 0.16).1);
    pb.line_to(p(0.76, 0.86).0, p(0.76, 0.86).1);
    pb.line_to(p(0.50, 0.66).0, p(0.50, 0.66).1);
    pb.line_to(p(0.24, 0.86).0, p(0.24, 0.86).1);
    pb.close();
    stroke_icon(canvas, pb.finish(), sw, color);
}

fn stroke_icon(canvas: &mut Pixmap, path: Option<Path>, width: f32, color: Color) {
    if let Some(path) = path {
        let mut p = Paint::default();
        p.set_color(color);
        p.anti_alias = true;
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Default::default()
        };
        canvas.stroke_path(&path, &p, &stroke, Transform::identity(), None);
    }
}

fn draw_heart_icon(canvas: &mut Pixmap, x: f32, y: f32, s: f32, sw: f32, color: Color) {
    let p = |nx: f32, ny: f32| (x + nx * s, y + ny * s);
    let mut pb = PathBuilder::new();
    let bot = p(0.50, 0.86);
    pb.move_to(bot.0, bot.1);
    let (c1, c2, l) = (p(0.14, 0.62), p(0.10, 0.34), p(0.24, 0.22));
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, l.0, l.1);
    let (c1, c2, m) = (p(0.36, 0.12), p(0.48, 0.22), p(0.50, 0.32));
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, m.0, m.1);
    let (c1, c2, r) = (p(0.52, 0.22), p(0.64, 0.12), p(0.76, 0.22));
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, r.0, r.1);
    let (c1, c2, end) = (p(0.90, 0.34), p(0.86, 0.62), bot);
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    pb.close();
    stroke_icon(canvas, pb.finish(), sw, color);
}

fn draw_comment_icon(canvas: &mut Pixmap, x: f32, y: f32, s: f32, sw: f32, color: Color) {
    let p = |nx: f32, ny: f32| (x + nx * s, y + ny * s);
    let mut pb = PathBuilder::new();
    let start = p(0.22, 0.78);
    pb.move_to(start.0, start.1);
    let tail = p(0.15, 0.88);
    pb.line_to(tail.0, tail.1);
    let rejoin = p(0.34, 0.83);
    pb.line_to(rejoin.0, rejoin.1);
    let (c1, c2, r) = (p(0.70, 0.90), p(0.90, 0.68), p(0.86, 0.45));
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, r.0, r.1);
    let (c1, c2, t) = (p(0.82, 0.20), p(0.52, 0.12), p(0.30, 0.20));
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, t.0, t.1);
    let (c1, c2, end) = (p(0.10, 0.29), p(0.11, 0.61), start);
    pb.cubic_to(c1.0, c1.1, c2.0, c2.1, end.0, end.1);
    stroke_icon(canvas, pb.finish(), sw, color);
}

fn draw_repost_icon(canvas: &mut Pixmap, x: f32, y: f32, s: f32, sw: f32, color: Color) {
    let p = |nx: f32, ny: f32| (x + nx * s, y + ny * s);
    let mut pb = PathBuilder::new();
    // Top-right arrow loop
    pb.move_to(p(0.20, 0.42).0, p(0.20, 0.42).1);
    pb.line_to(p(0.20, 0.26).0, p(0.20, 0.26).1);
    pb.line_to(p(0.74, 0.26).0, p(0.74, 0.26).1);
    pb.line_to(p(0.62, 0.14).0, p(0.62, 0.14).1);
    pb.move_to(p(0.74, 0.26).0, p(0.74, 0.26).1);
    pb.line_to(p(0.62, 0.38).0, p(0.62, 0.38).1);

    // Bottom-left return arrow
    pb.move_to(p(0.80, 0.58).0, p(0.80, 0.58).1);
    pb.line_to(p(0.80, 0.74).0, p(0.80, 0.74).1);
    pb.line_to(p(0.26, 0.74).0, p(0.26, 0.74).1);
    pb.line_to(p(0.38, 0.62).0, p(0.38, 0.62).1);
    pb.move_to(p(0.26, 0.74).0, p(0.26, 0.74).1);
    pb.line_to(p(0.38, 0.86).0, p(0.38, 0.86).1);
    stroke_icon(canvas, pb.finish(), sw, color);
}

fn draw_share_icon(canvas: &mut Pixmap, x: f32, y: f32, s: f32, sw: f32, color: Color) {
    let p = |nx: f32, ny: f32| (x + nx * s, y + ny * s);
    let mut pb = PathBuilder::new();
    pb.move_to(p(0.86, 0.18).0, p(0.86, 0.18).1);
    pb.line_to(p(0.14, 0.44).0, p(0.14, 0.44).1);
    pb.line_to(p(0.46, 0.54).0, p(0.46, 0.54).1);
    pb.line_to(p(0.58, 0.86).0, p(0.58, 0.86).1);
    pb.line_to(p(0.86, 0.18).0, p(0.86, 0.18).1);
    pb.move_to(p(0.86, 0.18).0, p(0.86, 0.18).1);
    pb.line_to(p(0.46, 0.54).0, p(0.46, 0.54).1);
    stroke_icon(canvas, pb.finish(), sw, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec(theme: ThreadsCardTheme) -> ThreadsCardSpec {
        ThreadsCardSpec {
            is_post: true,
            author_name: "Minh Anh".to_owned(),
            author_handle: "_mon.le09".to_owned(),
            timestamp: "3d".to_owned(),
            verified: false,
            badge: "Post".to_owned(),
            replying_to: String::new(),
            text: "Đơn siêu tốc, đặt giao quần áo khối lượng 5kg. Chạy lại thấy khách mang ra nguyên cái tủ lạnh... 🎨✨".to_owned(),
            likes: "34.8K".to_owned(),
            replies: "214".to_owned(),
            reposts: "1K".to_owned(),
            avatar: None,
            media: None,
            media_is_video: false,
            theme,
        }
    }

    #[test]
    fn renders_dark_and_light_threads_cards_in_portrait_frame() {
        let fonts = Fonts::new();
        for theme in [ThreadsCardTheme::Dark, ThreadsCardTheme::Light] {
            let rendered = render_threads_card(&fonts, &sample_spec(theme), 1080, 1920)
                .expect("card renders cleanly");
            assert_eq!(rendered.width, 1080);
            assert_eq!(rendered.height, 1920);
            assert_eq!(rendered.overscan, 1.0);
            assert!(rendered.block_width > 700);
            assert!(rendered.block_height > 200);
            assert!(rendered.block_height < 1400);
            assert!(rendered.png.starts_with(&[0x89, b'P', b'N', b'G']));
        }
    }

    #[test]
    fn renders_with_photo_avatar_and_clamps_long_text_inside_safe_zone() {
        let fonts = Fonts::new();
        let mut spec = sample_spec(ThreadsCardTheme::Dark);
        spec.is_post = false;
        spec.replying_to = "Trả lời @minhanh.dev".to_owned();
        spec.avatar = Some(AvatarPixels {
            width: 32,
            height: 32,
            rgba: vec![240; 32 * 32 * 4],
        });
        spec.text = "Đây là một bình luận cực kỳ dài về kinh nghiệm đi làm và những câu chuyện dở khóc dở cười trong văn phòng ".repeat(18);

        let rendered = render_threads_card(&fonts, &spec, 1080, 1920)
            .expect("clamped long comment renders cleanly");
        assert!((rendered.block_height as f32) <= 1920.0 * 0.74);

        // Card with embedded post photo + video badge fits cleanly inside portrait frame.
        let mut with_media = sample_spec(ThreadsCardTheme::Light);
        with_media.media = Some(AvatarPixels {
            width: 64,
            height: 48,
            rgba: vec![180; 64 * 48 * 4],
        });
        with_media.media_is_video = true;
        let rendered_media = render_threads_card(&fonts, &with_media, 1080, 1920)
            .expect("card with embedded media renders cleanly");
        assert!(rendered_media.block_height > 500);
        assert!((rendered_media.block_height as f32) <= 1920.0 * 0.74);
    }

    #[test]
    fn renders_opaque_backdrops_with_expected_theme_polarity() {
        let dark_png = render_threads_backdrop(ThreadsCardTheme::Dark, 240, 426)
            .expect("dark backdrop renders");
        let light_png = render_threads_backdrop(ThreadsCardTheme::Light, 240, 426)
            .expect("light backdrop renders");

        let dark_dec = png::Decoder::new(std::io::Cursor::new(dark_png.as_slice()));
        let mut dark_reader = dark_dec.read_info().expect("valid png");
        let mut dark_buf = vec![0; dark_reader.output_buffer_size().unwrap()];
        dark_reader.next_frame(&mut dark_buf).unwrap();

        let light_dec = png::Decoder::new(std::io::Cursor::new(light_png.as_slice()));
        let mut light_reader = light_dec.read_info().expect("valid png");
        let mut light_buf = vec![0; light_reader.output_buffer_size().unwrap()];
        light_reader.next_frame(&mut light_buf).unwrap();

        // Corner alpha must be opaque and light theme brighter than dark theme.
        assert_eq!(dark_buf[3], 255);
        assert_eq!(light_buf[3], 255);
        assert!(light_buf[0] > dark_buf[0] + 150);
    }
}
