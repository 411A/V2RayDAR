//! Desktop QR code sheet: LAN subscription + Telegram proxy in one image.
//!
//! The TUI `QR Codes: Generate & View` item (desktop only) collects the
//! shareable endpoints, renders them side by side into `QRCodes.jpg` inside
//! the `v2raydar_data` folder, and opens the image with the OS viewer.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use jpeg_encoder::{ColorType, Encoder};
use qrcodegen::{QrCode, QrCodeEcc};

use crate::{model::RuntimeConfig, network};

pub const QR_IMAGE_FILE_NAME: &str = "QRCodes.jpg";
const QR_IMAGE_JPEG_QUALITY: u8 = 90;

/// Fixed Full-HD sheet: every render is exactly 1920x1080. Cards share one
/// geometry and one module scale, so boxes and captions always line up no
/// matter how long each payload is.
pub const SHEET_WIDTH: u32 = 1920;
pub const SHEET_HEIGHT: u32 = 1080;

const TITLE_SCALE: u32 = 8;
const SUBTITLE_SCALE: u32 = 4;
const CAPTION_SCALE: u32 = 6;
const DETAIL_SCALE: u32 = 4;

const MARGIN_X: u32 = 80;
const TITLE_Y: u32 = 64;
const TITLE_SUB_GAP: u32 = 20;
const HEADER_CARD_GAP: u32 = 36;
const CARD_BOTTOM_MARGIN: u32 = 72;
const CARD_GAP: u32 = 72;
const CARD_BORDER: u32 = 8;
const CARD_PAD_X: u32 = 44;
const CARD_PAD_TOP: u32 = 44;
const CARD_PAD_BOTTOM: u32 = 44;
const CAPTION_GAP: u32 = 14;
/// The quiet zone is one module wider than the spec minimum so phone cameras
/// lock on even from a photo of the screen.
const QUIET_MODULES: u32 = 4;
const MIN_MODULE_PX: u32 = 6;
const GLYPH_WIDTH: u32 = 5;
const GLYPH_HEIGHT: u32 = 7;
const GLYPH_ADVANCE: u32 = GLYPH_WIDTH + 1;

const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
const INK: [u8; 3] = [0x11, 0x11, 0x11];
const SOFT_INK: [u8; 3] = [0x55, 0x55, 0x55];
const SUBSCRIPTION_ACCENT: [u8; 3] = [0x00, 0x96, 0x88];
const PROXY_ACCENT: [u8; 3] = [0x00, 0x88, 0xCC];

const SHEET_TITLE: &str = "V2RAYDAR";
const SHEET_SUBTITLE: &str = "SCAN WITH YOUR PHONE";
const SUBSCRIPTION_TITLE: &str = "LAN SUBSCRIPTION";
const PROXY_TITLE: &str = "TELEGRAM PROXY";

#[derive(Debug, Clone, Copy)]
struct TextStyle {
    scale: u32,
    color: [u8; 3],
    bold: bool,
}

const TITLE_STYLE: TextStyle = TextStyle {
    scale: TITLE_SCALE,
    color: INK,
    bold: true,
};
const SUBTITLE_STYLE: TextStyle = TextStyle {
    scale: SUBTITLE_SCALE,
    color: SOFT_INK,
    bold: false,
};
const CAPTION_STYLE: TextStyle = TextStyle {
    scale: CAPTION_SCALE,
    color: INK,
    bold: true,
};
const DETAIL_STYLE: TextStyle = TextStyle {
    scale: DETAIL_SCALE,
    color: SOFT_INK,
    bold: false,
};

/// One QR card on the sheet: what the phone reads plus how it is framed.
#[derive(Debug, Clone)]
pub struct QrCard {
    pub title: String,
    pub detail: String,
    pub text: String,
    pub accent: [u8; 3],
}

/// Planned sheet: renderable cards plus human reasons for anything skipped.
#[derive(Debug, Default)]
pub struct QrPlan {
    pub cards: Vec<QrCard>,
    pub skipped: Vec<String>,
}

/// Telegram deep link that opens a SOCKS5 entry pointing at this machine.
#[must_use]
pub fn telegram_proxy_url(host: &str, port: u16) -> String {
    format!("https://t.me/socks?server={host}&port={port}")
}

/// Plan the sheet from the live machine: real LAN discovery plus the
/// caller-supplied firewall check (production passes the firewall module's
/// `allows_port`).
#[must_use]
pub fn plan_live(config: &RuntimeConfig, firewall_ok: &dyn Fn(u16) -> bool) -> QrPlan {
    plan(
        config,
        &network::discoverable_hosts(config),
        network::primary_lan_ip().map(|ip| ip.to_string()),
        firewall_ok,
    )
}

/// Plan the sheet: subscription QR when sharing is on with a reachable LAN
/// host and an open firewall port, proxy QR when the proxy is LAN-enabled
/// under the same conditions. Anything missing lands in `skipped` with the
/// exact user-facing reason instead of silently vanishing.
///
/// `lan_hosts` comes from sharing-bind discovery; `proxy_fallback` covers a
/// LAN proxy paired with a loopback sharing bind (mirrors the status panel).
/// Tests pass both explicitly so they never touch the network.
#[must_use]
pub fn plan(
    config: &RuntimeConfig,
    lan_hosts: &[String],
    proxy_fallback: Option<String>,
    firewall_ok: &dyn Fn(u16) -> bool,
) -> QrPlan {
    let mut planned = QrPlan::default();
    plan_subscription(config, lan_hosts, firewall_ok, &mut planned);
    plan_proxy(config, lan_hosts, proxy_fallback, firewall_ok, &mut planned);
    planned
}

fn plan_subscription(
    config: &RuntimeConfig,
    lan_hosts: &[String],
    firewall_ok: &dyn Fn(u16) -> bool,
    planned: &mut QrPlan,
) {
    if !config.sharing_enabled {
        planned
            .skipped
            .push("LAN sharing is off (Main Menu -> Share subscription URL on LAN)".to_string());
        return;
    }
    let Some(host) = lan_hosts.first() else {
        planned
            .skipped
            .push("no reachable LAN IP found for the subscription".to_string());
        return;
    };
    let port = config.bind.port();
    if !firewall_ok(port) {
        planned.skipped.push(format!(
            "firewall is not allowing TCP {port} (subscription port)"
        ));
        return;
    }
    planned.cards.push(QrCard {
        title: SUBSCRIPTION_TITLE.to_string(),
        detail: host_port_detail(&config.subscription_url(host, true)),
        text: config.subscription_url(host, true),
        accent: SUBSCRIPTION_ACCENT,
    });
}

fn plan_proxy(
    config: &RuntimeConfig,
    lan_hosts: &[String],
    proxy_fallback: Option<String>,
    firewall_ok: &dyn Fn(u16) -> bool,
    planned: &mut QrPlan,
) {
    if !(config.proxy_enabled && config.proxy_discoverable) {
        planned.skipped.push(
            "proxy is not enabled for LAN (Main Menu -> Persistent proxy for app traffic)"
                .to_string(),
        );
        return;
    }
    let Some(host) = lan_hosts.first().cloned().or(proxy_fallback) else {
        planned
            .skipped
            .push("no reachable LAN IP found for the proxy".to_string());
        return;
    };
    let port = config.proxy_port;
    if !firewall_ok(port) {
        planned
            .skipped
            .push(format!("firewall is not allowing TCP {port} (proxy port)"));
        return;
    }
    planned.cards.push(QrCard {
        title: PROXY_TITLE.to_string(),
        detail: format!("{}:{port}", display_host(&host.to_ascii_uppercase())),
        text: telegram_proxy_url(&host, port),
        accent: PROXY_ACCENT,
    });
}

/// Short caption under a QR: `host:port` in the bitmap font's charset
/// (uppercase hex, bracketed IPv6), never the full URL with its token.
fn host_port_detail(url: &str) -> String {
    let fallback = url.to_ascii_uppercase();
    let Some(parsed) = url::Url::parse(url).ok() else {
        return fallback;
    };
    let Some(host) = parsed.host_str() else {
        return fallback;
    };
    let shown = display_host(&host.to_ascii_uppercase());
    match parsed.port() {
        Some(port) => format!("{shown}:{port}"),
        None => shown,
    }
}

/// IPv6 literals need brackets once a port is appended; IPv4 never does.
fn display_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// Render the planned cards side by side into a 1920x1080 JPEG image.
///
/// # Errors
///
/// Returns an error when no cards are given, a payload does not fit any QR
/// version or is too large for the sheet, or JPEG encoding fails.
pub fn render_jpeg(cards: &[QrCard]) -> Result<Vec<u8>> {
    if cards.is_empty() {
        bail!("no QR cards to render");
    }
    let mut codes = Vec::with_capacity(cards.len());
    for card in cards {
        let code = QrCode::encode_text(&card.text, QrCodeEcc::Medium)
            .map_err(|_| anyhow!("QR payload too long: {}", card.title))?;
        codes.push(code);
    }

    let counts: Vec<u32> = codes
        .iter()
        .map(|code| u32::try_from(code.size()).unwrap_or(0))
        .collect();
    let sheet = layout_sheet(&counts)?;

    let mut canvas = Canvas::new(SHEET_WIDTH, SHEET_HEIGHT)?;
    canvas.fill_rect(0, 0, SHEET_WIDTH, SHEET_HEIGHT, WHITE);
    canvas.draw_centered_text(SHEET_WIDTH, TITLE_Y, SHEET_TITLE, TITLE_STYLE);
    canvas.draw_centered_text(
        SHEET_WIDTH,
        TITLE_Y
            .saturating_add(glyph_block_height(TITLE_SCALE))
            .saturating_add(TITLE_SUB_GAP),
        SHEET_SUBTITLE,
        SUBTITLE_STYLE,
    );

    for (index, ((card, code), modules)) in cards
        .iter()
        .zip(codes.iter())
        .zip(counts.iter())
        .enumerate()
    {
        let slot = u32::try_from(index).unwrap_or(0);
        let x = sheet
            .first_card_x
            .saturating_add(slot.saturating_mul(sheet.card_w.saturating_add(CARD_GAP)));
        draw_card(&mut canvas, x, &sheet, card, code, *modules);
    }

    canvas.into_jpeg()
}

/// Fixed sheet geometry: uniform cards, one module scale for every QR, and
/// captions pinned to the same rows so boxes always align.
struct SheetLayout {
    card_w: u32,
    card_h: u32,
    card_y: u32,
    first_card_x: u32,
    module_px: u32,
    qr_zone_off_x: u32,
    qr_zone_off_y: u32,
    qr_zone_w: u32,
    qr_zone_h: u32,
    caption_y_off: u32,
    detail_y_off: u32,
}

fn layout_sheet(module_counts: &[u32]) -> Result<SheetLayout> {
    if module_counts.is_empty() {
        bail!("no QR cards to lay out");
    }
    // A lone card reuses the two-up card width instead of stretching wide.
    let slots = module_counts.len().max(2);
    let slots_u = u32::try_from(slots).unwrap_or(2).max(1);
    let card_y = TITLE_Y
        .saturating_add(glyph_block_height(TITLE_SCALE))
        .saturating_add(TITLE_SUB_GAP)
        .saturating_add(glyph_block_height(SUBTITLE_SCALE))
        .saturating_add(HEADER_CARD_GAP);
    let card_h = SHEET_HEIGHT
        .saturating_sub(card_y)
        .saturating_sub(CARD_BOTTOM_MARGIN);
    let gaps = CARD_GAP.saturating_mul(slots_u.saturating_sub(1));
    let card_w = SHEET_WIDTH
        .saturating_sub(MARGIN_X.saturating_mul(2))
        .saturating_sub(gaps)
        / slots_u;
    let caption_h = glyph_block_height(CAPTION_SCALE)
        .saturating_add(CAPTION_GAP)
        .saturating_add(glyph_block_height(DETAIL_SCALE));
    let qr_zone_w = card_w
        .saturating_sub(CARD_BORDER.saturating_mul(2))
        .saturating_sub(CARD_PAD_X.saturating_mul(2));
    let qr_zone_h = card_h
        .saturating_sub(CARD_BORDER.saturating_mul(2))
        .saturating_sub(CARD_PAD_TOP)
        .saturating_sub(caption_h)
        .saturating_sub(CARD_PAD_BOTTOM);
    let biggest = module_counts.iter().copied().max().unwrap_or(0);
    let span = biggest.saturating_add(QUIET_MODULES.saturating_mul(2));
    if span == 0 {
        bail!("QR matrix is empty");
    }
    let module_px = qr_zone_w.min(qr_zone_h) / span;
    if module_px < MIN_MODULE_PX {
        bail!("QR payload too large for the sheet");
    }
    let count_u = u32::try_from(module_counts.len()).unwrap_or(1).max(1);
    let row_w = card_w
        .saturating_mul(count_u)
        .saturating_add(CARD_GAP.saturating_mul(count_u.saturating_sub(1)));
    let caption_y_off = card_h
        .saturating_sub(CARD_BORDER)
        .saturating_sub(CARD_PAD_BOTTOM)
        .saturating_sub(glyph_block_height(DETAIL_SCALE))
        .saturating_sub(CAPTION_GAP)
        .saturating_sub(glyph_block_height(CAPTION_SCALE));
    Ok(SheetLayout {
        card_w,
        card_h,
        card_y,
        first_card_x: SHEET_WIDTH.saturating_sub(row_w) / 2,
        module_px,
        qr_zone_off_x: CARD_BORDER.saturating_add(CARD_PAD_X),
        qr_zone_off_y: CARD_BORDER.saturating_add(CARD_PAD_TOP),
        qr_zone_w,
        qr_zone_h,
        caption_y_off,
        detail_y_off: caption_y_off
            .saturating_add(glyph_block_height(CAPTION_SCALE))
            .saturating_add(CAPTION_GAP),
    })
}

fn draw_card(
    canvas: &mut Canvas,
    x: u32,
    sheet: &SheetLayout,
    card: &QrCard,
    code: &QrCode,
    modules: u32,
) {
    canvas.fill_rect(x, sheet.card_y, sheet.card_w, sheet.card_h, card.accent);
    canvas.fill_rect(
        x.saturating_add(CARD_BORDER),
        sheet.card_y.saturating_add(CARD_BORDER),
        sheet.card_w.saturating_sub(CARD_BORDER.saturating_mul(2)),
        sheet.card_h.saturating_sub(CARD_BORDER.saturating_mul(2)),
        WHITE,
    );

    let qr_side = modules
        .saturating_add(QUIET_MODULES.saturating_mul(2))
        .saturating_mul(sheet.module_px);
    let qr_x = x
        .saturating_add(sheet.qr_zone_off_x)
        .saturating_add(sheet.qr_zone_w.saturating_sub(qr_side) / 2);
    let qr_y = sheet
        .card_y
        .saturating_add(sheet.qr_zone_off_y)
        .saturating_add(sheet.qr_zone_h.saturating_sub(qr_side) / 2);
    draw_qr(canvas, qr_x, qr_y, code, sheet.module_px);

    canvas.draw_centered_text_in(
        x,
        sheet.card_w,
        sheet.card_y.saturating_add(sheet.caption_y_off),
        &card.title,
        CAPTION_STYLE,
    );
    canvas.draw_centered_text_in(
        x,
        sheet.card_w,
        sheet.card_y.saturating_add(sheet.detail_y_off),
        &card.detail,
        DETAIL_STYLE,
    );
}

/// Paint the QR matrix with a quiet zone; the caller owns positioning.
fn draw_qr(canvas: &mut Canvas, qr_x: u32, qr_y: u32, code: &QrCode, module_px: u32) {
    let quiet = QUIET_MODULES.saturating_mul(module_px);
    let origin_x = qr_x.saturating_add(quiet);
    let origin_y = qr_y.saturating_add(quiet);
    for row in 0..code.size() {
        for col in 0..code.size() {
            if !code.get_module(col, row) {
                continue;
            }
            let (Some(col), Some(row)) = (u32::try_from(col).ok(), u32::try_from(row).ok()) else {
                continue;
            };
            canvas.fill_rect(
                origin_x.saturating_add(col.saturating_mul(module_px)),
                origin_y.saturating_add(row.saturating_mul(module_px)),
                module_px,
                module_px,
                INK,
            );
        }
    }
}

struct Canvas {
    width: usize,
    height: usize,
    px: Vec<u8>,
}

impl Canvas {
    fn new(width: u32, height: u32) -> Result<Self> {
        let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else {
            bail!("QR sheet dimensions out of range");
        };
        let len = width
            .checked_mul(height)
            .and_then(|cells| cells.checked_mul(3))
            .context("QR sheet dimensions out of range")?;
        Ok(Self {
            width,
            height,
            px: vec![0xFF; len],
        })
    }

    fn put(&mut self, x: u32, y: u32, color: [u8; 3]) {
        let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        if x >= self.width || y >= self.height {
            return;
        }
        let Some(offset) = x
            .checked_add(y.saturating_mul(self.width))
            .and_then(|cell| cell.checked_mul(3))
        else {
            return;
        };
        if let Some(pixel) = self.px.get_mut(offset..offset.saturating_add(3)) {
            pixel.copy_from_slice(&color);
        }
    }

    fn fill_rect(&mut self, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
        for row in 0..height {
            for col in 0..width {
                self.put(x.saturating_add(col), y.saturating_add(row), color);
            }
        }
    }

    fn draw_centered_text(&mut self, row_width: u32, y: u32, text: &str, style: TextStyle) {
        let x = row_width.saturating_sub(text_width(text, style)) / 2;
        draw_text(self, x, y, text, style);
    }

    fn draw_centered_text_in(
        &mut self,
        x: u32,
        row_width: u32,
        y: u32,
        text: &str,
        style: TextStyle,
    ) {
        let offset = row_width.saturating_sub(text_width(text, style)) / 2;
        draw_text(self, x.saturating_add(offset), y, text, style);
    }

    fn into_jpeg(self) -> Result<Vec<u8>> {
        let (Ok(width), Ok(height)) = (u16::try_from(self.width), u16::try_from(self.height))
        else {
            bail!("QR sheet dimensions exceed JPEG limits");
        };
        let mut bytes = Vec::new();
        Encoder::new(&mut bytes, QR_IMAGE_JPEG_QUALITY)
            .encode(&self.px, width, height, ColorType::Rgb)
            .map_err(|error| anyhow!("JPEG encoding failed: {error}"))?;
        Ok(bytes)
    }
}

const fn glyph_block_height(scale: u32) -> u32 {
    GLYPH_HEIGHT.saturating_mul(scale)
}

fn text_width(text: &str, style: TextStyle) -> u32 {
    let glyphs = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    glyphs
        .saturating_mul(GLYPH_ADVANCE)
        .saturating_sub(1)
        .saturating_mul(style.scale)
        .saturating_add(u32::from(style.bold))
}

/// Bitmap text; bold overprints one pixel right for heavier stems. Unknown
/// characters advance blank so they can never corrupt the layout.
fn draw_text(canvas: &mut Canvas, x: u32, y: u32, text: &str, style: TextStyle) {
    draw_text_pass(canvas, x, y, text, style);
    if style.bold {
        draw_text_pass(canvas, x.saturating_add(1), y, text, style);
    }
}

fn draw_text_pass(canvas: &mut Canvas, x: u32, y: u32, text: &str, style: TextStyle) {
    let scale = style.scale;
    let color = style.color;
    let mut cursor = x;
    for ch in text.chars() {
        if let Some(rows) = glyph_rows(ch) {
            for (row, pattern) in rows.iter().enumerate() {
                for (col, filled) in pattern.chars().enumerate() {
                    if filled != '#' {
                        continue;
                    }
                    let (Ok(row), Ok(col)) = (u32::try_from(row), u32::try_from(col)) else {
                        continue;
                    };
                    canvas.fill_rect(
                        cursor.saturating_add(col.saturating_mul(scale)),
                        y.saturating_add(row.saturating_mul(scale)),
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        cursor = cursor.saturating_add(GLYPH_ADVANCE.saturating_mul(scale));
    }
}

/// Hand-drawn 5x7 bitmap font covering exactly what the sheet prints:
/// uppercase, digits, space, and `. : [ ]`. Unknown input renders as blank
/// advance so a stray character can never corrupt the layout.
const fn glyph_rows(ch: char) -> Option<&'static [&'static str; 7]> {
    match ch {
        'A'..='Z' => glyph_letter(ch),
        _ => glyph_symbol(ch),
    }
}

const fn glyph_letter(ch: char) -> Option<&'static [&'static str; 7]> {
    match ch {
        'A' => Some(&[
            ".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ]),
        'B' => Some(&[
            "####.", "#...#", "#...#", "####.", "#...#", "#...#", "####.",
        ]),
        'C' => Some(&[
            ".####", "#....", "#....", "#....", "#....", "#....", ".####",
        ]),
        'D' => Some(&[
            "####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####.",
        ]),
        'E' => Some(&[
            "#####", "#....", "#....", "####.", "#....", "#....", "#####",
        ]),
        'F' => Some(&[
            "#####", "#....", "#....", "####.", "#....", "#....", "#....",
        ]),
        'G' => Some(&[
            ".####", "#....", "#....", "#.###", "#...#", "#...#", ".###.",
        ]),
        'H' => Some(&[
            "#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#",
        ]),
        'I' => Some(&[
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####",
        ]),
        'J' => Some(&[
            "..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##..",
        ]),
        'K' => Some(&[
            "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#",
        ]),
        'L' => Some(&[
            "#....", "#....", "#....", "#....", "#....", "#....", "#####",
        ]),
        'M' => Some(&[
            "#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#",
        ]),
        'N' => Some(&[
            "#...#", "##..#", "##..#", "#.#.#", "#..##", "#..##", "#...#",
        ]),
        'O' => Some(&[
            ".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ]),
        'P' => Some(&[
            "####.", "#...#", "#...#", "####.", "#....", "#....", "#....",
        ]),
        'Q' => Some(&[
            ".###.", "#...#", "#...#", "#...#", "#...#", "#..#.", ".##.#",
        ]),
        'R' => Some(&[
            "####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#",
        ]),
        'S' => Some(&[
            ".####", "#....", "#....", ".###.", "....#", "....#", "####.",
        ]),
        'T' => Some(&[
            "#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#..",
        ]),
        'U' => Some(&[
            "#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###.",
        ]),
        'V' => Some(&[
            "#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#..",
        ]),
        'W' => Some(&[
            "#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#",
        ]),
        'X' => Some(&[
            "#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#",
        ]),
        'Y' => Some(&[
            "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#..",
        ]),
        'Z' => Some(&[
            "#####", "....#", "...#.", "..#..", ".#...", "#....", "#####",
        ]),
        _ => None,
    }
}

const fn glyph_symbol(ch: char) -> Option<&'static [&'static str; 7]> {
    match ch {
        '0' => Some(&[
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ]),
        '1' => Some(&[
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ]),
        '2' => Some(&[
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ]),
        '3' => Some(&[
            "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
        ]),
        '4' => Some(&[
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ]),
        '5' => Some(&[
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ]),
        '6' => Some(&[
            ".###.", "#....", "#....", "####.", "#...#", "#...#", ".###.",
        ]),
        '7' => Some(&[
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ]),
        '8' => Some(&[
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ]),
        '9' => Some(&[
            ".###.", "#...#", "#...#", ".####", "....#", "....#", ".###.",
        ]),
        ' ' => Some(&[
            ".....", ".....", ".....", ".....", ".....", ".....", ".....",
        ]),
        '.' => Some(&[
            ".....", ".....", ".....", ".....", ".....", ".##..", ".##..",
        ]),
        ':' => Some(&[
            ".....", ".##..", ".##..", ".....", ".##..", ".##..", ".....",
        ]),
        '[' => Some(&[
            ".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ]),
        ']' => Some(&[
            ".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###.",
        ]),
        _ => None,
    }
}

/// What `generate_and_save` produced: the saved sheet plus the reasons for
/// any card that did not qualify, so the caller can tell the user why a
/// card is missing instead of showing a silently partial sheet.
#[derive(Debug)]
pub struct QrOutcome {
    /// Where the sheet was written.
    pub path: PathBuf,
    /// Human reasons for cards that did not qualify. Non-empty on partial
    /// success (e.g. proxy rendered but the subscription port is firewalled);
    /// callers should surface these so a missing card never looks silent.
    pub skipped: Vec<String>,
}

/// Plan, render, and save the sheet as `QRCodes.jpg` in `v2raydar_data`.
///
/// # Errors
///
/// Returns an error describing why no card qualified (sharing off, no LAN
/// host, firewall closed), when rendering fails, or when the file cannot be
/// written.
pub fn generate_and_save(
    config: &RuntimeConfig,
    state_dir: &Path,
    firewall_ok: &dyn Fn(u16) -> bool,
) -> Result<QrOutcome> {
    let planned = plan_live(config, firewall_ok);
    save_planned(&planned, state_dir)
}

/// Persist an already-made plan; keeps the skip reasons so partial sheets
/// stay explainable. Split from `generate_and_save` so tests can cover the
/// partial-success path without touching the network.
fn save_planned(planned: &QrPlan, state_dir: &Path) -> Result<QrOutcome> {
    if planned.cards.is_empty() {
        bail!("QR Codes unavailable: {}", planned.skipped.join("; "));
    }
    let path = save_jpeg(&planned.cards, state_dir)?;
    Ok(QrOutcome {
        path,
        skipped: planned.skipped.clone(),
    })
}

/// Save rendered cards as `QRCodes.jpg` inside `v2raydar_data`.
///
/// # Errors
///
/// Returns an error when no cards are given, rendering fails, or the file
/// cannot be written.
pub fn save_jpeg(cards: &[QrCard], state_dir: &Path) -> Result<PathBuf> {
    if cards.is_empty() {
        bail!("no QR cards to save");
    }
    let jpeg = render_jpeg(cards)?;
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("unable to create {}", state_dir.display()))?;
    let path = state_dir.join(QR_IMAGE_FILE_NAME);
    std::fs::write(&path, jpeg).with_context(|| format!("unable to write {}", path.display()))?;
    Ok(path)
}

/// Open the saved sheet with the OS default image viewer.
#[must_use]
pub fn open_image(path: &Path) -> String {
    #[cfg(target_os = "windows")]
    {
        if super::tui::open_config::try_spawn(
            "cmd",
            &[
                "/C".to_string(),
                "start".to_string(),
                String::new(),
                super::tui::open_config::path_arg(path),
            ],
        )
        .is_ok()
        {
            return format!("QR image opened: {}", path.display());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if super::tui::open_config::try_spawn("open", &[super::tui::open_config::path_arg(path)])
            .is_ok()
        {
            return format!("QR image opened: {}", path.display());
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if super::tui::open_config::try_spawn(
            "xdg-open",
            &[super::tui::open_config::path_arg(path)],
        )
        .is_ok()
        {
            return format!("QR image opened: {}", path.display());
        }
    }

    format!("QR image saved (no viewer found): {}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn test_config() -> RuntimeConfig {
        RuntimeConfig {
            bind: "0.0.0.0:27141"
                .parse::<SocketAddr>()
                .expect("test bind parses"),
            top_n: 20,
            refresh_seconds: 900,
            ping_seconds: 300,
            encoded_subscription: false,
            prioritize_stability: true,
            return_configs_asap: true,
            scan_all_configs: false,
            fetch_timeout_ms: 30_000,
            fetch_concurrency: 8,
            max_subscription_bytes: 8_000_000,
            sharing_enabled: true,
            require_token: false,
            token: String::new(),
            probe_mode: "balanced".to_string(),
            speedtest_enabled: false,
            probe_concurrency: 16,
            probe_batch_size: None,
            active_timeout_ms: 30_000,
            startup_timeout_ms: 30_000,
            test_url: "https://www.gstatic.com/generate_204".to_string(),
            accepted_statuses: vec![200, 204],
            download_bytes_limit: 1_000_000,
            subscription_count: 1,
            enabled_subscription_count: 1,
            proxy_enabled: true,
            proxy_port: 27_910,
            proxy_discoverable: true,
        }
    }

    fn lan_hosts() -> Vec<String> {
        vec!["10.20.1.87".to_string()]
    }

    #[test]
    fn telegram_proxy_url_points_at_lan_socks() {
        assert_eq!(
            telegram_proxy_url("10.20.1.87", 27_910),
            "https://t.me/socks?server=10.20.1.87&port=27910"
        );
    }

    #[test]
    fn plan_includes_both_cards_when_everything_ready() {
        let planned = plan(&test_config(), &lan_hosts(), None, &|_| true);

        assert!(planned.skipped.is_empty());
        assert_eq!(planned.cards.len(), 2);
        assert_eq!(planned.cards[0].title, "LAN SUBSCRIPTION");
        assert_eq!(
            planned.cards[0].text,
            "http://10.20.1.87:27141/subscription.txt"
        );
        assert_eq!(planned.cards[0].detail, "10.20.1.87:27141");
        assert_eq!(planned.cards[1].title, "TELEGRAM PROXY");
        assert_eq!(
            planned.cards[1].text,
            "https://t.me/socks?server=10.20.1.87&port=27910"
        );
        assert_eq!(planned.cards[1].detail, "10.20.1.87:27910");
        assert_ne!(planned.cards[0].accent, planned.cards[1].accent);
    }

    #[test]
    fn plan_skips_subscription_when_sharing_off_but_keeps_proxy() {
        let mut config = test_config();
        config.sharing_enabled = false;

        let planned = plan(&config, &lan_hosts(), None, &|_| true);

        assert_eq!(planned.cards.len(), 1);
        assert_eq!(planned.cards[0].title, "TELEGRAM PROXY");
        assert_eq!(planned.skipped.len(), 1);
        assert!(planned.skipped[0].contains("sharing is off"));
    }

    #[test]
    fn plan_skips_everything_without_lan_host() {
        let planned = plan(&test_config(), &[], None, &|_| true);

        assert!(planned.cards.is_empty());
        assert_eq!(planned.skipped.len(), 2);
        assert!(
            planned
                .skipped
                .iter()
                .any(|reason| reason.contains("no reachable LAN IP"))
        );
    }

    #[test]
    fn plan_uses_proxy_fallback_when_sharing_hosts_are_empty() {
        let planned = plan(
            &test_config(),
            &[],
            Some("10.20.1.87".to_string()),
            &|_| true,
        );

        assert_eq!(planned.cards.len(), 1);
        assert_eq!(planned.cards[0].title, "TELEGRAM PROXY");
        assert_eq!(planned.cards[0].detail, "10.20.1.87:27910");
        assert_eq!(planned.skipped.len(), 1);
        assert!(planned.skipped[0].contains("no reachable LAN IP"));
    }

    #[test]
    fn plan_skips_card_whose_port_the_firewall_blocks() {
        let planned = plan(&test_config(), &lan_hosts(), None, &|port| port != 27_141);

        assert_eq!(planned.cards.len(), 1);
        assert_eq!(planned.cards[0].title, "TELEGRAM PROXY");
        assert!(
            planned.skipped[0].contains("27141"),
            "unexpected reasons: {:?}",
            planned.skipped
        );
    }

    #[test]
    fn plan_skips_proxy_when_it_is_not_lan_shared() {
        let mut config = test_config();
        config.proxy_discoverable = false;

        let planned = plan(&config, &lan_hosts(), None, &|_| true);

        assert_eq!(planned.cards.len(), 1);
        assert_eq!(planned.cards[0].title, "LAN SUBSCRIPTION");
        assert!(planned.skipped[0].contains("not enabled for LAN"));
    }

    #[test]
    fn subscription_detail_keeps_host_port_when_url_carries_a_token() {
        let mut config = test_config();
        config.token = "secret".to_string();

        let planned = plan(&config, &lan_hosts(), None, &|_| true);

        assert!(
            planned.cards[0].text.contains("token=secret"),
            "token should reach the phone: {}",
            planned.cards[0].text
        );
        assert_eq!(planned.cards[0].detail, "10.20.1.87:27141");
    }

    #[test]
    fn proxy_detail_brackets_ipv6_hosts() {
        let hosts = vec!["fe80::1".to_string()];

        let planned = plan(&test_config(), &hosts, None, &|_| true);

        assert_eq!(planned.cards[1].detail, "[FE80::1]:27910");
        assert!(
            planned.cards[1].text.contains("server=fe80::1"),
            "the link itself keeps the raw host: {}",
            planned.cards[1].text
        );
    }

    #[test]
    fn glyph_table_is_well_formed() {
        for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .:[]".chars() {
            let Some(rows) = glyph_rows(ch) else {
                panic!("missing glyph for {ch:?}");
            };
            assert_eq!(rows.len(), 7, "glyph {ch:?} must be 7 rows tall");
            for row in rows {
                assert_eq!(row.len(), 5, "glyph {ch:?} row {row:?} must be 5 wide");
                assert!(
                    row.chars().all(|cell| cell == '#' || cell == '.'),
                    "glyph {ch:?} row {row:?} uses only '#' and '.'"
                );
            }
        }
        assert_eq!(glyph_rows('?'), None);
    }

    #[test]
    fn every_planned_string_renders_without_unknown_glyphs() {
        let strings = [
            SHEET_TITLE,
            SHEET_SUBTITLE,
            SUBSCRIPTION_TITLE,
            PROXY_TITLE,
            "10.20.1.87:27141",
            "10.20.1.87:27910",
            "[FE80::1]:27910",
        ];
        for text in strings {
            assert!(
                text.chars().all(|ch| glyph_rows(ch).is_some()),
                "unrenderable string: {text:?}"
            );
            assert!(text_width(text, DETAIL_STYLE) > 0);
        }
    }

    #[test]
    fn encoded_qr_matrix_has_finder_corners() {
        let code = QrCode::encode_text(
            "http://10.20.1.87:27141/subscription.txt",
            QrCodeEcc::Medium,
        )
        .expect("sample payload encodes");
        let size = code.size();

        assert!((21..=177).contains(&size), "unexpected size {size}");
        assert_eq!(size % 2, 1, "QR size is always odd");
        assert!(code.get_module(0, 0));
        assert!(code.get_module(size - 1, 0));
        assert!(code.get_module(0, size - 1));
        assert!(!code.get_module(7, 7), "separator stays light");
    }

    fn sample_cards() -> Vec<QrCard> {
        plan(&test_config(), &lan_hosts(), None, &|_| true).cards
    }

    #[test]
    fn render_produces_a_valid_deterministic_jpeg() {
        let cards = sample_cards();

        let first = render_jpeg(&cards).expect("renders");
        let second = render_jpeg(&cards).expect("renders again");

        assert!(first.starts_with(&[0xFF, 0xD8, 0xFF]), "SOI marker");
        assert!(first.ends_with(&[0xFF, 0xD9]), "EOI marker");
        let expected = (
            u16::try_from(SHEET_WIDTH).expect("sheet width fits JPEG"),
            u16::try_from(SHEET_HEIGHT).expect("sheet height fits JPEG"),
        );
        assert_eq!(
            jpeg_dimensions(&first),
            Some(expected),
            "fixed Full-HD sheet"
        );
        assert!(first.len() > 20_000, "Full-HD sheet carries real pixels");
        assert_eq!(first, second, "same input renders byte-identical");
    }

    #[test]
    fn render_single_card_stays_full_hd_and_centered() {
        let single = render_jpeg(&sample_cards()[..1]).expect("single renders");
        let double = render_jpeg(&sample_cards()).expect("double renders");
        let expected = (
            u16::try_from(SHEET_WIDTH).expect("sheet width fits JPEG"),
            u16::try_from(SHEET_HEIGHT).expect("sheet height fits JPEG"),
        );

        assert_eq!(jpeg_dimensions(&single), Some(expected));
        assert_eq!(jpeg_dimensions(&double), Some(expected));
        assert_ne!(single, double, "one card differs from two");

        let one = layout_sheet(&[33]).expect("single lays out");
        let two = layout_sheet(&[33, 33]).expect("double lays out");
        assert_eq!(one.card_w, two.card_w, "lone card reuses two-up width");
        assert_eq!(
            one.first_card_x,
            (SHEET_WIDTH.saturating_sub(one.card_w)) / 2,
            "lone card centers on the sheet"
        );
        assert_eq!(
            two.first_card_x,
            (SHEET_WIDTH.saturating_sub(two.card_w.saturating_mul(2).saturating_add(CARD_GAP))) / 2,
            "the pair centers on the sheet"
        );
    }

    #[test]
    fn layout_shares_one_scale_and_pins_captions_for_mixed_sizes() {
        let sheet = layout_sheet(&[25, 41]).expect("mixed sizes lay out");

        assert!(sheet.module_px >= MIN_MODULE_PX);
        // The scale fits the biggest matrix: (41 + 2*4) * scale <= zone.
        assert!(
            (41 + QUIET_MODULES.saturating_mul(2)).saturating_mul(sheet.module_px)
                <= sheet.qr_zone_w.min(sheet.qr_zone_h)
        );
        assert!(sheet.card_h > sheet.qr_zone_h);
        assert!(sheet.caption_y_off < sheet.detail_y_off);
    }

    #[test]
    fn layout_rejects_empty_and_oversized_payloads() {
        assert!(layout_sheet(&[]).is_err());
        assert!(layout_sheet(&[200]).is_err());
    }

    /// Read image dimensions from the JPEG SOF0 marker without a decoder.
    fn jpeg_dimensions(bytes: &[u8]) -> Option<(u16, u16)> {
        let mut index: usize = 2;
        while index.saturating_add(9) < bytes.len() {
            if bytes.get(index) != Some(&0xFF) {
                index = index.saturating_add(1);
                continue;
            }
            let marker = bytes.get(index.saturating_add(1)).copied().unwrap_or(0);
            if marker == 0xC0 {
                let height = u16::from_be_bytes([
                    bytes.get(index.saturating_add(5)).copied().unwrap_or(0),
                    bytes.get(index.saturating_add(6)).copied().unwrap_or(0),
                ]);
                let width = u16::from_be_bytes([
                    bytes.get(index.saturating_add(7)).copied().unwrap_or(0),
                    bytes.get(index.saturating_add(8)).copied().unwrap_or(0),
                ]);
                return Some((width, height));
            }
            if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
                index = index.saturating_add(2);
                continue;
            }
            let len = usize::from(u16::from_be_bytes([
                bytes.get(index.saturating_add(2)).copied().unwrap_or(0),
                bytes.get(index.saturating_add(3)).copied().unwrap_or(0),
            ]));
            index = index.saturating_add(2).saturating_add(len);
        }
        None
    }

    #[test]
    fn render_rejects_an_empty_plan() {
        assert!(render_jpeg(&[]).is_err());
    }

    fn unique_temp_dir(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("v2raydar-qr-{case}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir creates");
        dir
    }

    #[test]
    fn save_jpeg_writes_qrcodes_jpg() {
        let dir = unique_temp_dir("save");

        let path = save_jpeg(&sample_cards(), &dir).expect("saves");

        assert_eq!(path, dir.join(QR_IMAGE_FILE_NAME));
        let bytes = std::fs::read(&path).expect("saved file reads");
        assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_jpeg_rejects_an_empty_plan() {
        let dir = unique_temp_dir("save-empty");

        let error = save_jpeg(&[], &dir).expect_err("must fail");

        assert!(format!("{error:#}").contains("no QR cards"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_planned_keeps_the_firewalled_subscription_reason() {
        // The reported bug: sharing on, proxy on, but only the proxy card
        // renders because the subscription port is firewalled. The outcome
        // must still carry the exact reason instead of going silent.
        let planned = plan(&test_config(), &lan_hosts(), None, &|port| port != 27_141);
        assert_eq!(planned.cards.len(), 1);
        let dir = unique_temp_dir("partial-skip");

        let outcome = save_planned(&planned, &dir).expect("proxy card still renders");

        assert_eq!(outcome.path, dir.join(QR_IMAGE_FILE_NAME));
        assert!(outcome.path.exists());
        assert_eq!(outcome.skipped.len(), 1);
        assert!(
            outcome.skipped[0].contains("27141"),
            "unexpected reasons: {:?}",
            outcome.skipped
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_planned_reports_no_skips_when_everything_ready() {
        let planned = plan(&test_config(), &lan_hosts(), None, &|_| true);
        assert!(planned.skipped.is_empty());
        let dir = unique_temp_dir("no-skip");

        let outcome = save_planned(&planned, &dir).expect("renders");

        assert!(outcome.skipped.is_empty());
        assert!(outcome.path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_and_save_explains_an_empty_plan() {
        let mut config = test_config();
        config.sharing_enabled = false;
        config.proxy_enabled = false;
        let dir = unique_temp_dir("empty");

        let error = generate_and_save(&config, &dir, &|_| true).expect_err("must fail");

        let message = format!("{error:#}");
        assert!(message.contains("sharing is off"), "{message}");
        assert!(message.contains("not enabled for LAN"), "{message}");
        assert!(!dir.join(QR_IMAGE_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
