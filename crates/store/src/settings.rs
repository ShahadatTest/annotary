//! Member settings (CMD-23): the one catalog both clients render from, the hybrid logical clock
//! stamps every stored value carries, and the highlight colors with their contrast rules. Pure:
//! the storage and the Device record are `devices.rs`.
//!
//! - **User settings** have one value per Member. **Device settings** have a value per Device
//!   plus an **All devices** value; a Device resolves to its own value, else All devices, else the
//!   default ([`Source`]).
//! - [`CATALOG`] lists every setting: key, kind, label, help line (searched, never shown under the
//!   name), group and order on the Settings screen, type and allowed values, default, and which
//!   Device kinds it applies to (who counts toward Mixed). The server checks every write against
//!   it ([`validate`]); `GET /api/v1/me/settings/catalog` serves [`catalog_json`].
//! - A [`Stamp`] is `MS-COUNTER-WRITER`: wall time in milliseconds, a counter, the writer's id (the
//!   server's `s<tenant id>`, or a Device's id). Stamps compare in that order, so the order is
//!   total. The server keeps its clock in `settings` (`hlc`) and refuses a stamp more than
//!   [`MAX_AHEAD_MS`] ahead of its wall clock.

use std::cmp::Ordering;

use serde_json::{Value, json};

/// User or device setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    /// One value per Member, the same on every Device.
    User,
    /// A value per Device, plus the All devices value.
    Device,
}

impl SettingKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SettingKind::User => "user",
            SettingKind::Device => "device",
        }
    }
}

/// What values a setting takes.
#[derive(Clone, Copy, Debug)]
pub enum ValueType {
    /// One of the listed `(value, label)` strings.
    Choice(&'static [(&'static str, &'static str)]),
    /// One of the listed integers (Text size, in percent).
    Steps(&'static [i64]),
    /// A [`PRESETS`] key or a custom `#rrggbb`; `dark` says which ground it is checked on.
    Color { dark: bool },
    /// `"any"` or a Vault id (checked against the store, not the catalog).
    Vault,
    /// The Member's display name (stored on `members`, written through rename).
    Name,
}

impl ValueType {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueType::Choice(_) => "choice",
            ValueType::Steps(_) => "steps",
            ValueType::Color { .. } => "color",
            ValueType::Vault => "vault",
            ValueType::Name => "name",
        }
    }
}

/// One group of the Settings screen's sidebar, in order. Slugs are never `server` or `vaults`
/// (the owner pages and their fragments live under `/settings/server/…` and `/settings/vaults/…`).
#[derive(Clone, Copy, Debug)]
pub struct GroupDef {
    pub slug: &'static str,
    pub label: &'static str,
}

/// The groups, in sidebar order. Notifications stays out until it has a setting.
pub const GROUPS: &[GroupDef] = &[
    GroupDef { slug: "profile", label: "Profile" },
    GroupDef { slug: "devices", label: "Devices" },
    GroupDef { slug: "appearance", label: "Appearance" },
    GroupDef { slug: "threads", label: "Threads" },
    GroupDef { slug: "documents", label: "Documents" },
    GroupDef { slug: "keyboard", label: "Keyboard" },
    GroupDef { slug: "date-and-time", label: "Date and time" },
];

/// One setting.
#[derive(Clone, Copy, Debug)]
pub struct SettingDef {
    pub key: &'static str,
    pub kind: SettingKind,
    pub label: &'static str,
    /// Matched by the Settings search; never shown under the name.
    pub help: &'static str,
    pub group: &'static str,
    pub order: u32,
    pub ty: ValueType,
    /// The built-in default, as JSON text.
    pub default: &'static str,
    /// Device kinds a device setting applies to (who counts toward Mixed); empty for user ones.
    pub device_kinds: &'static [&'static str],
}

impl SettingDef {
    pub fn default_value(&self) -> Value {
        serde_json::from_str(self.default).expect("catalog defaults are JSON")
    }

    pub fn applies_to(&self, kind: &str) -> bool {
        self.device_kinds.contains(&kind)
    }
}

/// Device kinds a Device row may have.
pub const DEVICE_KINDS: &[&str] = &["iphone", "ipad", "web", "cli"];
const SCREENS: &[&str] = &["web", "iphone", "ipad"];
const KEYBOARDS: &[&str] = &["web", "ipad"];

/// Text size steps, in percent (100% is the web UI's 15 px Document text).
pub const TEXT_SIZES: &[i64] = &[80, 90, 100, 115, 130, 150, 175, 200];

/// Every setting. Starter set of CMD-23; a later setting needs only its entry here and the
/// clients' behavior.
pub const CATALOG: &[SettingDef] = &[
    SettingDef {
        key: "display_name",
        kind: SettingKind::User,
        label: "Display name",
        help: "Your name, shown next to your Comments.",
        group: "profile",
        order: 1,
        ty: ValueType::Name,
        default: "\"\"",
        device_kinds: &[],
    },
    SettingDef {
        key: "appearance",
        kind: SettingKind::Device,
        label: "Appearance",
        help: "Light or dark pages, or follow the system.",
        group: "appearance",
        order: 1,
        ty: ValueType::Choice(&[("system", "Follow system"), ("light", "Light"), ("dark", "Dark")]),
        default: "\"system\"",
        device_kinds: SCREENS,
    },
    SettingDef {
        key: "highlight_light",
        kind: SettingKind::Device,
        label: "Highlight, light",
        help: "The color of highlighted text on light pages.",
        group: "appearance",
        order: 2,
        ty: ValueType::Color { dark: false },
        default: "\"yellow\"",
        device_kinds: SCREENS,
    },
    SettingDef {
        key: "highlight_dark",
        kind: SettingKind::Device,
        label: "Highlight, dark",
        help: "The color of highlighted text on dark pages.",
        group: "appearance",
        order: 3,
        ty: ValueType::Color { dark: true },
        default: "\"yellow\"",
        device_kinds: SCREENS,
    },
    SettingDef {
        key: "text_size",
        kind: SettingKind::Device,
        label: "Text size",
        help: "The size of Document text.",
        group: "appearance",
        order: 4,
        ty: ValueType::Steps(TEXT_SIZES),
        default: "100",
        device_kinds: SCREENS,
    },
    SettingDef {
        key: "show_resolved",
        kind: SettingKind::User,
        label: "Resolved Threads",
        help: "Show or hide resolved Threads in Documents.",
        group: "threads",
        order: 1,
        ty: ValueType::Choice(&[("hidden", "Hidden"), ("shown", "Shown")]),
        default: "\"hidden\"",
        device_kinds: &[],
    },
    SettingDef {
        key: "default_vault",
        kind: SettingKind::User,
        label: "Default Vault",
        help: "Where new Documents go when you don't name a Vault.",
        group: "documents",
        order: 1,
        ty: ValueType::Vault,
        default: "\"any\"",
        device_kinds: &[],
    },
    SettingDef {
        key: "keyboard_shortcuts",
        kind: SettingKind::Device,
        label: "Keyboard shortcuts",
        help: "Single-key shortcuts in Documents (? lists them).",
        group: "keyboard",
        order: 1,
        ty: ValueType::Choice(&[("on", "On"), ("off", "Off")]),
        default: "\"on\"",
        device_kinds: KEYBOARDS,
    },
    SettingDef {
        key: "send_key",
        kind: SettingKind::Device,
        label: "Send key for replies",
        help: "Which key posts a reply.",
        group: "keyboard",
        order: 2,
        ty: ValueType::Choice(&[("enter", "↩ posts"), ("mod_enter", "⌘↩ posts")]),
        default: "\"enter\"",
        device_kinds: KEYBOARDS,
    },
    SettingDef {
        key: "times",
        kind: SettingKind::User,
        label: "Times shown as",
        help: "Relative times (2 hours ago) or dates and times.",
        group: "date-and-time",
        order: 1,
        ty: ValueType::Choice(&[("relative", "Relative"), ("absolute", "Absolute")]),
        default: "\"relative\"",
        device_kinds: &[],
    },
    SettingDef {
        key: "clock",
        kind: SettingKind::User,
        label: "Clock",
        help: "12-hour or 24-hour times, or follow the system.",
        group: "date-and-time",
        order: 2,
        ty: ValueType::Choice(&[("system", "Follow system"), ("12", "12-hour"), ("24", "24-hour")]),
        default: "\"system\"",
        device_kinds: &[],
    },
];

/// The catalog entry for `key`.
pub fn setting(key: &str) -> Option<&'static SettingDef> {
    CATALOG.iter().find(|d| d.key == key)
}

/// Device settings, in catalog order.
pub fn device_settings() -> impl Iterator<Item = &'static SettingDef> {
    CATALOG.iter().filter(|d| d.kind == SettingKind::Device)
}

// ---------------------------------------------------------------- highlight colors

/// One highlight preset: its light and dark color and the derived selected shades. Dark
/// presets are bright, so highlighted text in dark is drawn dark ([`DARK_HL_TEXT`]).
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub key: &'static str,
    pub name: &'static str,
    pub light: &'static str,
    pub light_selected: &'static str,
    pub dark: &'static str,
    pub dark_selected: &'static str,
}

/// The presets, in swatch order. Yellow light is the pre-CMD-23 `--cd-hl` / `--cd-hl-sel`.
pub const PRESETS: &[Preset] = &[
    Preset {
        key: "yellow",
        name: "Yellow",
        light: "#fff1a8",
        light_selected: "#ffd54a",
        dark: "#facc15",
        dark_selected: "#fde047",
    },
    Preset {
        key: "green",
        name: "Green",
        light: "#c8f2d0",
        light_selected: "#86e29b",
        dark: "#4ade80",
        dark_selected: "#86efac",
    },
    Preset {
        key: "blue",
        name: "Blue",
        light: "#cfe3ff",
        light_selected: "#93c1ff",
        dark: "#60a5fa",
        dark_selected: "#93c5fd",
    },
    Preset {
        key: "pink",
        name: "Pink",
        light: "#fbd3e9",
        light_selected: "#f4a6cf",
        dark: "#f472b6",
        dark_selected: "#f9a8d4",
    },
    Preset {
        key: "orange",
        name: "Orange",
        light: "#fed7aa",
        light_selected: "#fdba74",
        dark: "#fb923c",
        dark_selected: "#fdba74",
    },
    Preset {
        key: "purple",
        name: "Purple",
        light: "#e4d4fb",
        light_selected: "#c9a9f6",
        dark: "#c084fc",
        dark_selected: "#d8b4fe",
    },
];

/// Text on a highlight on a light page (the web UI's `--cd-fg`).
pub const LIGHT_HL_TEXT: &str = "#1c1c1e";
/// Text on a highlight on a dark page: dark, as on a real highlighter.
pub const DARK_HL_TEXT: &str = "#000000";
/// Refined Threads' own color, light and dark (they also keep a dotted underline).
pub const REFINED_LIGHT: &str = "#c7d2fe";
pub const REFINED_DARK: &str = "#818cf8";
/// WCAG AA for body text.
pub const MIN_CONTRAST: f64 = 4.5;
/// How far (CIE76 ΔE) every preset stays from the refined color of its appearance. The approved
/// light Purple (9.6) and Blue (10.6) are the closest; refined also keeps its dotted underline,
/// so a refined mark never depends on color alone.
pub const MIN_REFINED_DELTA_E: f64 = 9.0;

/// `#rrggbb` (any case) → (r, g, b).
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn channel(v: u8) -> f64 {
    let c = v as f64 / 255.0;
    if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn luminance((r, g, b): (u8, u8, u8)) -> f64 {
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// WCAG contrast ratio of two `#rrggbb` colors (1.0 when either does not parse).
pub fn contrast(a: &str, b: &str) -> f64 {
    let (Some(a), Some(b)) = (parse_hex(a), parse_hex(b)) else { return 1.0 };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn lab((r, g, b): (u8, u8, u8)) -> (f64, f64, f64) {
    let (r, g, b) = (channel(r), channel(g), channel(b));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// CIE76 color difference of two `#rrggbb` colors (0.0 when either does not parse).
pub fn delta_e(a: &str, b: &str) -> f64 {
    let (Some(a), Some(b)) = (parse_hex(a), parse_hex(b)) else { return 0.0 };
    let (l1, a1, b1) = lab(a);
    let (l2, a2, b2) = lab(b);
    ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
}

fn mix(c: (u8, u8, u8), toward: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let m = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * t).round().clamp(0.0, 255.0) as u8;
    (m(c.0, toward.0), m(c.1, toward.1), m(c.2, toward.2))
}

/// A highlight value (preset key or `#rrggbb`) → `(color, selected shade)` for a light or dark
/// page. A custom color's selected shade is derived: darker on light pages, lighter on dark.
/// An unknown value falls back to Yellow.
pub fn highlight_colors(value: &str, dark: bool) -> (String, String) {
    if let Some(p) = PRESETS.iter().find(|p| p.key == value) {
        return if dark {
            (p.dark.to_string(), p.dark_selected.to_string())
        } else {
            (p.light.to_string(), p.light_selected.to_string())
        };
    }
    match parse_hex(value) {
        Some(c) if dark => (hex(c), hex(mix(c, (255, 255, 255), 0.3))),
        Some(c) => (hex(c), hex(mix(c, (0, 0, 0), 0.15))),
        None => highlight_colors("yellow", dark),
    }
}

// ---------------------------------------------------------------- validation

/// Why a value was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// Not a key of the catalog (`400 unknown_setting`).
    UnknownSetting(String),
    /// Outside the catalog's allowed values (`400 invalid_setting`).
    Invalid { key: &'static str, reason: String },
    /// A custom highlight whose text falls below 4.5:1 (`400 low_contrast`).
    LowContrast { key: &'static str, ratio: f64 },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::UnknownSetting(k) => write!(f, "no setting {k:?}"),
            Refusal::Invalid { key, reason } => write!(f, "{key}: {reason}"),
            Refusal::LowContrast { key, ratio } => {
                write!(f, "{key}: text on that color has a contrast of {ratio:.2}:1, below {MIN_CONTRAST}:1")
            }
        }
    }
}

/// Check `value` against `def` and return it normalized (colors lowercased). A Vault id is only
/// checked for shape here; the store checks it exists.
pub fn validate(def: &SettingDef, value: &Value) -> Result<Value, Refusal> {
    let bad = |reason: String| Refusal::Invalid { key: def.key, reason };
    match def.ty {
        ValueType::Choice(opts) => {
            let s = value.as_str().ok_or_else(|| bad(format!("use {}", choice_words(opts))))?;
            if opts.iter().any(|(v, _)| *v == s) {
                Ok(json!(s))
            } else {
                Err(bad(format!("{s:?} is not one of {}", choice_words(opts))))
            }
        }
        ValueType::Steps(steps) => {
            let n = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|s| s.trim().trim_end_matches('%').parse().ok()))
                .ok_or_else(|| bad(format!("use one of {}", steps_words(steps))))?;
            if steps.contains(&n) {
                Ok(json!(n))
            } else {
                Err(bad(format!("{n} is not one of {}", steps_words(steps))))
            }
        }
        ValueType::Color { dark } => {
            let s = value.as_str().ok_or_else(|| bad("use a preset name or #rrggbb".into()))?.trim().to_lowercase();
            if PRESETS.iter().any(|p| p.key == s) {
                return Ok(json!(s));
            }
            if parse_hex(&s).is_none() {
                let names: Vec<&str> = PRESETS.iter().map(|p| p.key).collect();
                return Err(bad(format!("{s:?} is not {} or #rrggbb", names.join(", "))));
            }
            let text = if dark { DARK_HL_TEXT } else { LIGHT_HL_TEXT };
            let ratio = contrast(&s, text);
            if ratio < MIN_CONTRAST {
                return Err(Refusal::LowContrast { key: def.key, ratio: (ratio * 100.0).round() / 100.0 });
            }
            Ok(json!(s))
        }
        ValueType::Vault => {
            let s = value.as_str().ok_or_else(|| bad("use \"any\" or a Vault id".into()))?.trim();
            if s.is_empty() {
                return Err(bad("use \"any\" or a Vault id".into()));
            }
            Ok(json!(s))
        }
        ValueType::Name => {
            let s = value.as_str().ok_or_else(|| bad("use a name".into()))?.trim();
            if s.is_empty() || s.chars().any(char::is_control) {
                return Err(bad("a name is not empty and has no control characters".into()));
            }
            Ok(json!(s))
        }
    }
}

fn choice_words(opts: &[(&str, &str)]) -> String {
    opts.iter().map(|(v, _)| *v).collect::<Vec<_>>().join(", ")
}

fn steps_words(steps: &[i64]) -> String {
    steps.iter().map(i64::to_string).collect::<Vec<_>>().join(", ")
}

/// The catalog as `GET /api/v1/me/settings/catalog` serves it: `{groups[], settings[], presets[]}`.
pub fn catalog_json() -> Value {
    let groups: Vec<Value> =
        GROUPS.iter().enumerate().map(|(i, g)| json!({ "slug": g.slug, "label": g.label, "order": i + 1 })).collect();
    let settings: Vec<Value> = CATALOG.iter().map(setting_json).collect();
    let presets: Vec<Value> = PRESETS
        .iter()
        .map(|p| {
            json!({ "key": p.key, "name": p.name, "light": p.light, "light_selected": p.light_selected,
                    "dark": p.dark, "dark_selected": p.dark_selected })
        })
        .collect();
    json!({
        "groups": groups,
        "settings": settings,
        "presets": presets,
        "highlight_text": { "light": LIGHT_HL_TEXT, "dark": DARK_HL_TEXT },
        "refined": { "light": REFINED_LIGHT, "dark": REFINED_DARK },
    })
}

/// One catalog entry as JSON.
pub fn setting_json(d: &SettingDef) -> Value {
    let mut v = json!({
        "key": d.key, "kind": d.kind.as_str(), "label": d.label, "help": d.help, "group": d.group,
        "order": d.order, "type": d.ty.as_str(), "default": d.default_value(), "device_kinds": d.device_kinds,
    });
    match d.ty {
        ValueType::Choice(opts) => {
            v["options"] = opts.iter().map(|(val, label)| json!({ "value": val, "label": label })).collect();
        }
        ValueType::Steps(steps) => v["steps"] = json!(steps),
        ValueType::Color { dark } => v["appearance"] = json!(if dark { "dark" } else { "light" }),
        ValueType::Vault | ValueType::Name => {}
    }
    v
}

// ---------------------------------------------------------------- stamps

/// Longest a pushed stamp may run ahead of the server's wall clock.
pub const MAX_AHEAD_MS: u64 = 5 * 60 * 1000;

/// A hybrid logical clock stamp, `MS-COUNTER-WRITER`. Ordered by wall time, then counter, then
/// writer, so two stamps from different writers always differ.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Stamp {
    pub ms: u64,
    pub counter: u32,
    pub writer: String,
}

impl Stamp {
    /// Parse `MS-COUNTER-WRITER` (writer: 1–64 ASCII letters, digits or `_`).
    pub fn parse(s: &str) -> Option<Stamp> {
        let mut it = s.trim().splitn(3, '-');
        let ms = it.next()?.parse().ok()?;
        let counter = it.next()?.parse().ok()?;
        let writer = it.next()?;
        if writer.is_empty() || writer.len() > 64 || !writer.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        Some(Stamp { ms, counter, writer: writer.to_string() })
    }

    /// True when `self` comes after `other` on the clock alone (ignoring the writer).
    pub fn clock_after(&self, other: &Stamp) -> bool {
        (self.ms, self.counter) > (other.ms, other.counter)
    }
}

impl std::fmt::Display for Stamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}-{}", self.ms, self.counter, self.writer)
    }
}

impl Ord for Stamp {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.ms, self.counter, &self.writer).cmp(&(other.ms, other.counter, &other.writer))
    }
}

impl PartialOrd for Stamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The wall clock in milliseconds since the epoch.
pub fn wall_ms() -> u64 {
    jiff::Timestamp::now().as_millisecond().max(0) as u64
}

// ---------------------------------------------------------------- device ids

/// Crockford base32, the alphabet of a Device id.
pub const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Length of a client-made Device id.
pub const DEVICE_ID_LEN: usize = 26;
/// The one fixed Device every browser is.
pub const WEB_DEVICE: &str = "web";

/// A Device id a client may name: `web`, or 26 characters of Crockford base32 (upper case).
pub fn is_device_id(s: &str) -> bool {
    s == WEB_DEVICE || (s.len() == DEVICE_ID_LEN && s.bytes().all(|b| CROCKFORD.contains(&b)))
}

/// A new random Device id (what iOS makes once; tests and the CLI use it too).
pub fn new_device_id() -> String {
    let mut buf = [0u8; DEVICE_ID_LEN];
    crate::ids::random_bytes(&mut buf);
    buf.iter().map(|b| CROCKFORD[(b % 32) as usize] as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_order_by_ms_then_counter_then_writer() {
        let a = Stamp::parse("1000-0-sA").unwrap();
        let b = Stamp::parse("1000-1-sA").unwrap();
        let c = Stamp::parse("1000-1-sB").unwrap();
        let d = Stamp::parse("1001-0-sA").unwrap();
        assert!(a < b && b < c && c < d);
        assert_eq!(b.to_string(), "1000-1-sA");
        assert!(Stamp::parse("x-1-a").is_none());
        assert!(Stamp::parse("1-1-").is_none());
        assert!(Stamp::parse("1-1-a-b").is_none(), "'-' is not a writer character");
    }

    #[test]
    fn device_ids_are_web_or_26_crockford() {
        assert!(is_device_id("web"));
        let id = new_device_id();
        assert!(is_device_id(&id), "{id}");
        assert!(!is_device_id(&id.to_lowercase()));
        assert!(!is_device_id("01JB0000000000000000000000I"));
        assert!(!is_device_id("short"));
        assert!(!is_device_id("01JB000000000000000000000U"), "U is not Crockford");
    }

    #[test]
    fn group_slugs_never_shadow_the_owner_routes() {
        for g in GROUPS {
            assert!(g.slug != "server" && g.slug != "vaults", "{}", g.slug);
        }
        for d in CATALOG {
            assert!(GROUPS.iter().any(|g| g.slug == d.group), "{} has an unknown group", d.key);
            validate(d, &d.default_value())
                .or_else(|e| if d.key == "display_name" { Ok(json!("")) } else { Err(e) })
                .unwrap();
        }
    }
}
