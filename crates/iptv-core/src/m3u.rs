//! Extended M3U parser (the format used by iptv-org and most IPTV providers).
//!
//! Supported:
//! * `#EXTINF:<duration> key="value" ...,<title>` with `tvg-id`, `tvg-name`,
//!   `tvg-logo`, `group-title` (`;`-separated), `http-user-agent`,
//!   `http-referrer`
//! * `#EXTVLCOPT:http-user-agent=...` / `#EXTVLCOPT:http-referrer=...`
//! * `#EXTGRP:<group>`
//! * bare URL lines without a preceding `#EXTINF`

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub name: String,
    pub url: String,
    pub tvg_id: Option<String>,
    pub logo: Option<String>,
    pub groups: Vec<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
}

pub fn parse(input: &str) -> Vec<Channel> {
    let input = input.trim_start_matches('\u{feff}');
    let mut out = Vec::new();
    let mut pending: Option<Channel> = None;

    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending = Some(parse_extinf(rest));
        } else if let Some(rest) = line.strip_prefix("#EXTVLCOPT:") {
            if let (Some(ch), Some((k, v))) = (pending.as_mut(), rest.split_once('=')) {
                let v = v.trim().to_string();
                match k.trim() {
                    "http-user-agent" => ch.user_agent = Some(v),
                    "http-referrer" | "http-referer" => ch.referrer = Some(v),
                    _ => {}
                }
            }
        } else if let Some(rest) = line.strip_prefix("#EXTGRP:") {
            if let Some(ch) = pending.as_mut() {
                if ch.groups.is_empty() {
                    ch.groups = split_groups(rest);
                }
            }
        } else if line.starts_with('#') {
            // #EXTM3U header or unsupported directive.
        } else {
            let mut ch = pending.take().unwrap_or_default();
            ch.url = line.to_string();
            if ch.name.is_empty() {
                ch.name = line.to_string();
            }
            out.push(ch);
        }
    }
    out
}

fn split_groups(s: &str) -> Vec<String> {
    s.split(';')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_string)
        .collect()
}

/// Parses everything after `#EXTINF:`.
///
/// All delimiters we look for are ASCII, so byte indices at those positions
/// are always valid UTF-8 boundaries.
fn parse_extinf(s: &str) -> Channel {
    let b = s.as_bytes();
    let mut ch = Channel::default();
    let mut tvg_name = None;
    let mut i = 0;

    // Duration token.
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b',' {
        i += 1;
    }

    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b',' {
            ch.name = s[i + 1..].trim().to_string();
            break;
        }
        let key_start = i;
        while i < b.len() && b[i] != b'=' && b[i] != b',' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = &s[key_start..i];
        if i < b.len() && b[i] == b'=' {
            i += 1;
            let value = if i < b.len() && b[i] == b'"' {
                i += 1;
                let start = i;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                let v = &s[start..i];
                if i < b.len() {
                    i += 1; // closing quote
                }
                v
            } else {
                let start = i;
                while i < b.len() && b[i] != b',' && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                &s[start..i]
            };
            let value = value.trim();
            let some = || (!value.is_empty()).then(|| value.to_string());
            match key.to_ascii_lowercase().as_str() {
                "tvg-id" => ch.tvg_id = some(),
                "tvg-name" => tvg_name = some(),
                "tvg-logo" => ch.logo = some(),
                "group-title" => ch.groups = split_groups(value),
                "http-user-agent" | "user-agent" => ch.user_agent = some(),
                "http-referrer" | "http-referer" | "referrer" => ch.referrer = some(),
                _ => {}
            }
        }
    }

    if ch.name.is_empty() {
        ch.name = tvg_name.unwrap_or_default();
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}#EXTM3U
#EXTINF:-1 tvg-id=\"00sReplay.us@SD\" tvg-logo=\"https://x/logo.png\" group-title=\"Movies\",00s Replay
https://jmp2.uk/plu-62ba60f059624e000781c436.m3u8
#EXTINF:-1 tvg-id=\"1Plus1.ua@SD\" http-user-agent=\"Mozilla/5.0 (Windows NT 10.0; Win64; x64)\" group-title=\"General\",1+1 International
#EXTVLCOPT:http-referrer=https://example.com/
http://158.101.222.193:88/georgia_play.php?id=1plus1international
#EXTINF:-1 group-title=\"Entertainment;Family;General\",Name, with comma (576p)
http://a/b.m3u8

http://bare/url.m3u8
";

    #[test]
    fn parses_iptv_org_style_playlist() {
        let chs = parse(SAMPLE);
        assert_eq!(chs.len(), 4);

        assert_eq!(chs[0].name, "00s Replay");
        assert_eq!(chs[0].tvg_id.as_deref(), Some("00sReplay.us@SD"));
        assert_eq!(chs[0].logo.as_deref(), Some("https://x/logo.png"));
        assert_eq!(chs[0].groups, vec!["Movies"]);
        assert_eq!(chs[0].url, "https://jmp2.uk/plu-62ba60f059624e000781c436.m3u8");

        assert_eq!(chs[1].name, "1+1 International");
        assert_eq!(
            chs[1].user_agent.as_deref(),
            Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        );
        assert_eq!(chs[1].referrer.as_deref(), Some("https://example.com/"));

        assert_eq!(chs[2].name, "Name, with comma (576p)");
        assert_eq!(chs[2].groups, vec!["Entertainment", "Family", "General"]);

        assert_eq!(chs[3].name, "http://bare/url.m3u8");
        assert!(chs[3].groups.is_empty());
    }

    #[test]
    fn falls_back_to_tvg_name_and_survives_garbage() {
        let chs = parse("#EXTINF:-1 tvg-name=\"Fallback\" broken= \"x,\nhttp://u\n#EXTINF:\nhttp://v\n");
        assert_eq!(chs.len(), 2);
        assert_eq!(chs[0].name, "Fallback");
        assert_eq!(chs[1].name, "http://v");
    }
}
