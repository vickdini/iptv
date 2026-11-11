//! Moving favorites between devices: a small JSON file listing each
//! favorite's name and stream address.
//!
//! ```json
//! {"iptv": "favorites", "version": 1,
//!  "favorites": [{"name": "Rai 1", "url": "https://..."}]}
//! ```
//!
//! Importing adds to the favorites already there. Channels are found by
//! address, or by name when the playlist's address changed since.

use serde::{Deserialize, Serialize};

use crate::{i18n::t, tuner::normalize};

const KIND: &str = "favorites";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    iptv: String,
    version: u32,
    favorites: Vec<Entry>,
}

/// The file's text for these favorites.
pub fn to_file(entries: Vec<Entry>) -> String {
    let file = File {
        iptv: KIND.into(),
        version: VERSION,
        favorites: entries,
    };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Reads a favorites file; the error is a message for the user.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let file: File = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| t("This isn't an IPTV favorites file.").to_string())?;
    if file.iptv != KIND {
        return Err(t("This isn't an IPTV favorites file.").into());
    }
    if file.version > VERSION {
        return Err(t("This favorites file is from a newer IPTV: update this one first.").into());
    }
    Ok(file
        .favorites
        .into_iter()
        .filter(|e| !e.url.is_empty())
        .collect())
}

/// The stream addresses to add: each entry's channel in this playlist (by
/// address, else by name), or its own address if the playlist doesn't have
/// it (kept, so it counts once the channel is back). Also returns how many
/// entries weren't found in this playlist.
pub fn resolve(entries: &[Entry], names: &[String], urls: &[String]) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut missing = 0;
    for e in entries {
        let by_url = urls.iter().position(|u| *u == e.url);
        let by_name = || {
            let want = normalize(e.name.trim());
            (!want.is_empty())
                .then(|| names.iter().position(|n| normalize(n.trim()) == want))
                .flatten()
        };
        match by_url.or_else(by_name) {
            Some(i) => out.push(urls[i].clone()),
            None => {
                missing += 1;
                out.push(e.url.clone());
            }
        }
    }
    out.dedup();
    (out, missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, url: &str) -> Entry {
        Entry {
            name: name.into(),
            url: url.into(),
        }
    }

    #[test]
    fn round_trip_and_matching() {
        let text = to_file(vec![
            e("Rai 1", "http://a"),
            e("Télé Sud", "http://old-b"),
            e("Gone", "http://x"),
        ]);
        let entries = parse(&text).unwrap();
        assert_eq!(entries.len(), 3);
        let names = vec!["Rai 1".to_string(), "Tele sud".to_string()];
        let urls = vec!["http://a".to_string(), "http://new-b".to_string()];
        let (add, missing) = resolve(&entries, &names, &urls);
        // By address, by name (the address changed), and kept as is.
        assert_eq!(add, ["http://a", "http://new-b", "http://x"]);
        assert_eq!(missing, 1);
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse("{\"hello\": 1}").is_err());
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"iptv":"favorites","version":9,"favorites":[]}"#).is_err());
    }
}
