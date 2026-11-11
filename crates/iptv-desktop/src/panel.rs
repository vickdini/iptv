//! Channel list panel drawn as an mpv ASS overlay (`osd-overlay`).
//!
//! The overlay uses a virtual canvas `H` units tall; its width follows the
//! window aspect ratio (`res_x = 0`). The panel occupies the left `PANEL_W`
//! units, so it keeps the same proportions at any window size.

use iptv_core::{tuner::normalize, Browser, Tuner};

/// Virtual canvas height.
pub const H: f64 = 720.0;
const PANEL_W: f64 = 600.0;
const HEADER_H: f64 = 62.0;
const ROW_H: f64 = 28.0;
const FOOTER_H: f64 = 34.0;
const PAD: f64 = 16.0;
const NUM_W: f64 = 64.0;
/// Room for each channel's logo, left of its name.
const LOGO_W: f64 = 40.0;
const LOGO_X: f64 = PAD + NUM_W + 10.0;
const SCROLLBAR_W: f64 = 8.0;
const SCROLLBAR_HIT_W: f64 = 24.0;
pub const ROWS: usize = ((H - HEADER_H - FOOTER_H) / ROW_H) as usize;

// ASS colors are &HBBGGRR&.
const C_TEXT: &str = "FFFFFF";
const C_DIM: &str = "B4B4B4";
const C_PLAYING: &str = "40D0FF"; // amber
const C_SELECT: &str = "A05A1E"; // blue
const C_BG: &str = "141414";

pub enum Hit {
    Row(usize),
    /// Scrollbar track, as a fraction of its height.
    Scrollbar(f64),
    /// Inside the panel but not on a row (header/footer).
    Panel,
    Outside,
}

pub struct Panel {
    pub browser: Browser,
}

impl Panel {
    pub fn new(tuner: &Tuner) -> Self {
        Self {
            browser: Browser::new(tuner.view(), tuner.current(), ROWS),
        }
    }

    /// Rebuilds the item list after the tuner's filter changed.
    pub fn refresh(&mut self, tuner: &Tuner) {
        self.browser.set_items(tuner.view(), tuner.current());
    }

    /// Where each visible row's logo goes: (row, channel, x, y, w, h) in
    /// canvas units.
    pub fn logo_slots(&self) -> Vec<(usize, usize, f64, f64, f64, f64)> {
        self.browser
            .visible()
            .map(|(row, ch, _)| {
                let top = HEADER_H + row as f64 * ROW_H;
                (row, ch, LOGO_X, top + 3.0, LOGO_W, ROW_H - 6.0)
            })
            .collect()
    }

    /// Hit-test a point in virtual canvas coordinates.
    pub fn hit(&self, x: f64, y: f64) -> Hit {
        hit_list(x, y, self.browser.len())
    }

    /// `status` (e.g. "checking channels 40%") replaces the footer hints.
    /// `typing`: the search being typed (shown with a cursor), if any.
    pub fn render(&self, tuner: &Tuner, status: Option<&str>, typing: Option<&str>) -> String {
        let mut ev: Vec<String> = Vec::new();
        let b = &self.browser;

        ev.push(rect(0.0, 0.0, PANEL_W, H, C_BG, 0x30));
        let (title, hint) = match typing {
            Some(q) => (
                format!("Search: {q}_"),
                "type a channel name      Enter  done      Esc  clear",
            ),
            None => (
                tuner.view_title(),
                "/  search     c  countries     g / G  recent, groups     S  favorites",
            ),
        };
        ev.push(text(PAD, 24.0, 26, C_TEXT, true, &title, PANEL_W - PAD));
        ev.push(text(PAD, 48.0, 15, C_DIM, false, hint, PANEL_W - PAD));
        ev.push(rect(
            PAD,
            HEADER_H - 4.0,
            PANEL_W - PAD,
            HEADER_H - 3.0,
            C_DIM,
            0x90,
        ));

        if b.is_empty() {
            let empty = if tuner.search_text().is_some() {
                "No channels found"
            } else {
                "No channels here"
            };
            ev.push(text(
                PAD,
                HEADER_H + ROW_H,
                20,
                C_DIM,
                false,
                empty,
                PANEL_W,
            ));
        }

        let name_x = LOGO_X + LOGO_W + 8.0;
        let name_right = PANEL_W - SCROLLBAR_HIT_W;
        for (row, ch, selected) in b.visible() {
            let top = HEADER_H + row as f64 * ROW_H;
            let mid = top + ROW_H / 2.0;
            if selected {
                ev.push(rect(
                    4.0,
                    top + 1.0,
                    PANEL_W - SCROLLBAR_HIT_W + 4.0,
                    top + ROW_H - 1.0,
                    C_SELECT,
                    0x20,
                ));
            }
            let playing = ch == tuner.current();
            let color = if playing { C_PLAYING } else { C_TEXT };
            ev.push(format!(
                "{{\\an6\\pos({},{mid})\\fs18\\bord0\\shad0\\1c&H{C_DIM}&}}{}",
                PAD + NUM_W,
                ch + 1
            ));
            let mut name = String::new();
            if playing {
                name.push_str("▶ ");
            }
            name.push_str(&tuner.channels()[ch].name);
            if tuner.is_favorite(ch) {
                name.push_str("  ★");
            }
            ev.push(text(name_x, mid, 20, color, playing, &name, name_right));
        }

        // Scrollbar.
        if b.len() > ROWS {
            let top = HEADER_H;
            let height = ROWS as f64 * ROW_H;
            let x0 = PANEL_W - PAD / 2.0 - SCROLLBAR_W;
            ev.push(rect(x0, top, x0 + SCROLLBAR_W, top + height, C_DIM, 0xC0));
            let thumb_h = (height * ROWS as f64 / b.len() as f64).max(24.0);
            let max_scroll = (b.len() - ROWS) as f64;
            let thumb_y = top + (height - thumb_h) * b.scroll() as f64 / max_scroll;
            ev.push(rect(
                x0,
                thumb_y,
                x0 + SCROLLBAR_W,
                thumb_y + thumb_h,
                C_TEXT,
                0x40,
            ));
        }

        let first = b.scroll() + 1;
        let last = (b.scroll() + ROWS).min(b.len());
        let footer = match (status, tuner.region_label()) {
            (Some(status), _) => format!("{first}–{last} of {}     {status}…", b.len()),
            (None, Some(region)) => format!("{first}–{last} of {}     showing: {region}", b.len()),
            (None, None) => format!(
                "{first}–{last} of {}     wheel scroll · click / Enter watch · Esc close",
                b.len()
            ),
        };
        ev.push(text(
            PAD,
            H - FOOTER_H / 2.0,
            15,
            C_DIM,
            false,
            &footer,
            PANEL_W - PAD,
        ));

        ev.join("\n")
    }
}

/// Hit-test for a list drawn with the panel's geometry (`len` rows in all).
fn hit_list(x: f64, y: f64, len: usize) -> Hit {
    if !(0.0..=PANEL_W).contains(&x) || !(0.0..=H).contains(&y) {
        return Hit::Outside;
    }
    let list_top = HEADER_H;
    let list_bottom = HEADER_H + ROWS as f64 * ROW_H;
    if !(list_top..list_bottom).contains(&y) {
        return Hit::Panel;
    }
    if x >= PANEL_W - SCROLLBAR_HIT_W && len > ROWS {
        return Hit::Scrollbar((y - list_top) / (list_bottom - list_top));
    }
    Hit::Row(((y - list_top) / ROW_H) as usize)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTab {
    Countries,
    Languages,
}

/// Country / language chooser, drawn in place of the channel list.
/// Rows have a check box, the name and the number of working channels;
/// typing narrows the list. While anything is chosen, the first row clears
/// all filters.
///
/// Rows are numbered for the list model: 0 is the "clear all" row, and the
/// country / language `i` is row `i + 1` (so rows stay in ascending order).
pub struct Picker {
    pub tab: PickerTab,
    pub query: String,
    all: Vec<(String, usize)>,
    /// Something is chosen (in either tab): offer the "clear all" row.
    has_filter: bool,
    pub browser: Browser,
}

const CLEAR_ROW: usize = 0;

impl Picker {
    pub fn new(tuner: &Tuner) -> Self {
        let mut p = Self {
            tab: PickerTab::Countries,
            query: String::new(),
            all: Vec::new(),
            has_filter: false,
            browser: Browser::new(vec![], 0, ROWS),
        };
        p.reload(tuner);
        p
    }

    /// After ticking: refreshes the rows, keeping the selection where it is.
    pub fn update(&mut self, tuner: &Tuner) {
        self.has_filter = tuner.region_label().is_some();
        let items = self.rows();
        self.browser.replace_items(items);
    }

    fn rows(&self) -> Vec<usize> {
        let q = normalize(self.query.trim());
        let clear = (self.has_filter && q.is_empty()).then_some(CLEAR_ROW);
        clear
            .into_iter()
            .chain(
                (0..self.all.len())
                    .filter(|&i| normalize(&self.all[i].0).contains(&q))
                    .map(|i| i + 1),
            )
            .collect()
    }

    /// The "clear all filters" row is selected.
    pub fn clear_selected(&self) -> bool {
        self.browser.selected_channel() == Some(CLEAR_ROW)
    }

    /// Reloads names and counts for the current tab and re-applies the typed filter.
    pub fn reload(&mut self, tuner: &Tuner) {
        self.all = match self.tab {
            PickerTab::Countries => tuner.countries(),
            PickerTab::Languages => tuner.languages(),
        };
        self.has_filter = tuner.region_label().is_some();
        self.refilter();
    }

    pub fn refilter(&mut self) {
        let items = self.rows();
        self.browser.set_items(items, 0);
    }

    pub fn switch_tab(&mut self, tuner: &Tuner) {
        self.tab = match self.tab {
            PickerTab::Countries => PickerTab::Languages,
            PickerTab::Languages => PickerTab::Countries,
        };
        self.query.clear();
        self.reload(tuner);
    }

    pub fn selected_name(&self) -> Option<&str> {
        match self.browser.selected_channel()? {
            CLEAR_ROW => None,
            row => Some(self.all[row - 1].0.as_str()),
        }
    }

    pub fn hit(&self, x: f64, y: f64) -> Hit {
        hit_list(x, y, self.browser.len())
    }

    pub fn render(&self, tuner: &Tuner) -> String {
        let mut ev: Vec<String> = Vec::new();
        let b = &self.browser;
        let (countries, languages) = tuner.regions();
        let (chosen, name, other) = match self.tab {
            PickerTab::Countries => (&countries, "Countries", "languages"),
            PickerTab::Languages => (&languages, "Languages", "countries"),
        };

        ev.push(rect(0.0, 0.0, PANEL_W, H, C_BG, 0x30));
        let title = if self.query.is_empty() {
            match chosen.len() {
                0 => name.to_string(),
                n => format!("{name} · {n} chosen"),
            }
        } else {
            format!("{name}: {}_", self.query)
        };
        ev.push(text(PAD, 24.0, 26, C_TEXT, true, &title, PANEL_W - PAD));
        let hint = format!(
            "type to find     Enter  tick     Tab  {other}     Del  clear all     Esc  back"
        );
        ev.push(text(PAD, 48.0, 15, C_DIM, false, &hint, PANEL_W - PAD));
        ev.push(rect(
            PAD,
            HEADER_H - 4.0,
            PANEL_W - PAD,
            HEADER_H - 3.0,
            C_DIM,
            0x90,
        ));

        if self.all.is_empty() {
            ev.push(text(
                PAD,
                HEADER_H + ROW_H,
                20,
                C_DIM,
                false,
                "Not available for this playlist",
                PANEL_W,
            ));
        } else if b.is_empty() {
            ev.push(text(
                PAD,
                HEADER_H + ROW_H,
                20,
                C_DIM,
                false,
                "Nothing matches",
                PANEL_W,
            ));
        }

        let right = PANEL_W - SCROLLBAR_HIT_W;
        for (row, i, selected) in b.visible() {
            let top = HEADER_H + row as f64 * ROW_H;
            let mid = top + ROW_H / 2.0;
            if selected {
                ev.push(rect(
                    4.0,
                    top + 1.0,
                    PANEL_W - SCROLLBAR_HIT_W + 4.0,
                    top + ROW_H - 1.0,
                    C_SELECT,
                    0x20,
                ));
            }
            if i == CLEAR_ROW {
                ev.push(text(
                    PAD + 8.0,
                    mid,
                    20,
                    C_PLAYING,
                    true,
                    "✕  Clear all filters (show every channel)",
                    right,
                ));
                continue;
            }
            let (label, count) = &self.all[i - 1];
            let on = chosen.contains(label);
            let mark = if on { "☑" } else { "☐" };
            let color = if on { C_PLAYING } else { C_TEXT };
            ev.push(text(
                PAD + 8.0,
                mid,
                20,
                color,
                on,
                &format!("{mark}  {label}"),
                right - 70.0,
            ));
            ev.push(format!(
                "{{\\an6\\pos({},{mid})\\fs17\\bord0\\shad0\\1c&H{C_DIM}&}}{count}",
                right - 8.0
            ));
        }

        if b.len() > ROWS {
            let top = HEADER_H;
            let height = ROWS as f64 * ROW_H;
            let x0 = PANEL_W - PAD / 2.0 - SCROLLBAR_W;
            ev.push(rect(x0, top, x0 + SCROLLBAR_W, top + height, C_DIM, 0xC0));
            let thumb_h = (height * ROWS as f64 / b.len() as f64).max(24.0);
            let max_scroll = (b.len() - ROWS) as f64;
            let thumb_y = top + (height - thumb_h) * b.scroll() as f64 / max_scroll;
            ev.push(rect(
                x0,
                thumb_y,
                x0 + SCROLLBAR_W,
                thumb_y + thumb_h,
                C_TEXT,
                0x40,
            ));
        }

        let footer = match tuner.region_label() {
            Some(region) => format!("showing channels from / in: {region}"),
            None => "nothing chosen: showing all channels".to_string(),
        };
        ev.push(text(
            PAD,
            H - FOOTER_H / 2.0,
            15,
            C_DIM,
            false,
            &footer,
            PANEL_W - PAD,
        ));
        ev.join("\n")
    }
}

/// Filled rectangle. `alpha` 0 = opaque, 255 = invisible.
/// The phone remote card: a QR code to scan, the address and PIN to type
/// instead. `pairing` is None when this computer has no network address.
/// How a phone connects to the remote: what [`remote_card`] shows.
pub struct Pairing<'a> {
    pub address: &'a str,
    pub pin: &'a str,
    /// QR code width and its dark modules.
    pub qr: &'a (usize, Vec<bool>),
}

pub fn remote_card(pairing: Option<Pairing>) -> String {
    let (x0, y0, w) = (40.0, 40.0, 660.0);
    let x1 = x0 + w;
    let mut ev = Vec::new();
    let Some(Pairing {
        address,
        pin,
        qr: (size, dark),
    }) = pairing
    else {
        ev.push(rect(x0, y0, x1, y0 + 150.0, C_BG, 0x10));
        ev.push(text(
            x0 + PAD * 2.0,
            y0 + 45.0,
            30,
            C_TEXT,
            true,
            "Phone remote",
            x1,
        ));
        ev.push(text(
            x0 + PAD * 2.0,
            y0 + 95.0,
            22,
            C_DIM,
            false,
            "This computer isn't connected to a network.",
            x1,
        ));
        return ev.join("\n");
    };
    let module = 8.0;
    let quiet = 4.0; // white margin scanners need, in modules
    let side = (*size as f64 + 2.0 * quiet) * module;
    let qr_y = y0 + 80.0;
    let y1 = qr_y + side + 190.0;
    ev.push(rect(x0, y0, x1, y1, C_BG, 0x10));
    ev.push(text(
        x0 + PAD * 2.0,
        y0 + 42.0,
        30,
        C_TEXT,
        true,
        "Phone remote",
        x1,
    ));
    let qx = x0 + (w - side) / 2.0;
    ev.push(rect(qx, qr_y, qx + side, qr_y + side, "FFFFFF", 0));
    // Dark modules, one rectangle per horizontal run.
    let mut path = String::new();
    for row in 0..*size {
        let mut col = 0;
        while col < *size {
            if !dark[row * size + col] {
                col += 1;
                continue;
            }
            let start = col;
            while col < *size && dark[row * size + col] {
                col += 1;
            }
            let (ax, ay) = (
                qx + (start as f64 + quiet) * module,
                qr_y + (row as f64 + quiet) * module,
            );
            let (bx, by) = (qx + (col as f64 + quiet) * module, ay + module);
            path.push_str(&format!("m {ax} {ay} l {bx} {ay} {bx} {by} {ax} {by} "));
        }
    }
    ev.push(format!(
        "{{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H000000&\\1a&H00&\\p1}}{path}{{\\p0}}"
    ));
    let t = qr_y + side + 34.0;
    ev.push(text(
        x0 + PAD * 2.0,
        t,
        22,
        C_TEXT,
        false,
        "Scan with your phone's camera (same Wi-Fi as this computer),",
        x1 - PAD,
    ));
    ev.push(text(
        x0 + PAD * 2.0,
        t + 36.0,
        22,
        C_TEXT,
        false,
        &format!("or open  {address}"),
        x1 - PAD,
    ));
    ev.push(text(
        x0 + PAD * 2.0,
        t + 72.0,
        22,
        C_TEXT,
        false,
        &format!("and enter the PIN  {pin}"),
        x1 - PAD,
    ));
    ev.push(text(
        x0 + PAD * 2.0,
        t + 120.0,
        18,
        C_DIM,
        false,
        "Any key closes this card. The remote keeps working.",
        x1 - PAD,
    ));
    ev.join("\n")
}

/// A one-line note on a dark box at the bottom left (over the video).
pub fn hint(msg: &str) -> String {
    const SIZE: u32 = 24;
    let (x0, y1) = (32.0, H - 32.0);
    let y0 = y1 - 44.0;
    // Rough text width; the box only needs to be about right.
    let w = msg.chars().count() as f64 * SIZE as f64 * 0.46 + 2.0 * PAD;
    let x1 = x0 + w;
    [
        rect(x0, y0, x1, y1, C_BG, 0x40),
        text(x0 + PAD, (y0 + y1) / 2.0, SIZE, C_TEXT, false, msg, x1),
    ]
    .join("\n")
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64, color: &str, alpha: u8) -> String {
    format!(
        "{{\\an7\\pos(0,0)\\bord0\\shad0\\1c&H{color}&\\1a&H{alpha:02X}&\\p1}}m {x0} {y0} l {x1} {y0} {x1} {y1} {x0} {y1}{{\\p0}}"
    )
}

/// Left-aligned, vertically centered single-line text clipped at `right`.
fn text(x: f64, mid: f64, size: u32, color: &str, bold: bool, s: &str, right: f64) -> String {
    let half = size as f64;
    format!(
        "{{\\an4\\pos({x},{mid})\\fs{size}\\b{}\\bord1\\shad0\\3c&H000000&\\1c&H{color}&\\clip({x},{},{right},{})}}{}",
        bold as u8,
        mid - half,
        mid + half,
        escape(s)
    )
}

/// Escapes text so ASS doesn't interpret it as override tags.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\u{200B}"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_ass_tags() {
        assert_eq!(escape(r"a{\b1}b\n"), "a\\{\\\u{200B}b1\\}b\\\u{200B}n");
    }
}
