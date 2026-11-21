//! Saved playlist sources and their optional guides.
use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DEFAULT: &str = "https://iptv-org.github.io/iptv/index.m3u";
pub const UPLOAD_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    location: String,
    pub guide: Option<String>,
    #[serde(default)]
    file: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub active: String,
    pub entries: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Input {
    Select {
        id: String,
    },
    Url {
        name: String,
        url: String,
        guide: Option<String>,
    },
    File {
        name: String,
        text: String,
        guide: Option<String>,
    },
}

pub fn web_url(value: &str) -> bool {
    value.len() <= 4096
        && url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
}

impl Input {
    pub fn valid(&self) -> bool {
        let (name, guide) = match self {
            Self::Select { id } => return !id.is_empty() && id.len() <= 64,
            Self::Url { name, url, guide } if web_url(url) => (name, guide),
            Self::File { name, text, guide } if text.len() <= UPLOAD_LIMIT => (name, guide),
            _ => return false,
        };
        !name.trim().is_empty()
            && name.len() <= 160
            && !name.chars().any(char::is_control)
            && guide.as_deref().is_none_or(web_url)
    }
}

pub fn retain_web_channels(channels: &mut Vec<iptv_core::Channel>) -> Result<()> {
    channels.retain(|channel| web_url(&channel.url));
    if channels.is_empty() {
        bail!("The playlist contains no HTTP or HTTPS channels");
    }
    Ok(())
}

fn presets() -> Vec<Source> {
    [
        ("iptv-org", "iptv-org", DEFAULT),
        (
            "free-tv",
            "Free-TV/IPTV",
            "https://raw.githubusercontent.com/Free-TV/IPTV/master/playlist.m3u8",
        ),
        (
            "iptv-italy",
            "IPTV-Italy",
            "https://raw.githubusercontent.com/sfiorini/IPTV-Italy/main/iptvitaly.m3u",
        ),
    ]
    .into_iter()
    .map(|(id, name, url)| Source {
        id: id.into(),
        name: name.into(),
        location: url.into(),
        guide: None,
        file: false,
    })
    .collect()
}

impl Catalog {
    pub fn load(dir: &Path) -> Self {
        let mut catalog: Self = fs::read(dir.join("playlists.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| Self {
                active: "iptv-org".into(),
                entries: vec![],
            });
        for (index, preset) in presets().into_iter().enumerate() {
            if !catalog.entries.iter().any(|source| source.id == preset.id) {
                catalog
                    .entries
                    .insert(index.min(catalog.entries.len()), preset);
            }
        }
        if !catalog
            .entries
            .iter()
            .any(|source| source.id == catalog.active)
        {
            catalog.active = "iptv-org".into();
        }
        catalog
    }

    pub fn current(&self) -> &Source {
        self.entries
            .iter()
            .find(|source| source.id == self.active)
            .expect("loaded catalog")
    }

    pub fn public(&self) -> serde_json::Value {
        serde_json::json!({
            "active": self.active,
            "entries": self.entries.iter().map(|source| serde_json::json!({
                "id": source.id, "name": source.name
            })).collect::<Vec<_>>()
        })
    }

    pub fn set_guide(&mut self, guide: Option<String>) {
        if let Some(source) = self
            .entries
            .iter_mut()
            .find(|source| source.id == self.active)
        {
            source.guide = guide;
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let path = dir.join("playlists.json");
        let tmp = dir.join("playlists.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn select(&mut self, input: Input, dir: &Path) -> Result<()> {
        if !input.valid() {
            bail!("Invalid playlist source");
        }
        let (name, data, guide, file) = match input {
            Input::Select { id } => {
                if !self.entries.iter().any(|entry| entry.id == id) {
                    bail!("Unknown playlist");
                }
                self.active = id;
                return Ok(());
            }
            Input::Url { name, url, guide } => (name, url, guide, false),
            Input::File { name, text, guide } => (name, text, guide, true),
        };
        let mut digest = Sha256::new();
        digest.update([u8::from(file)]);
        digest.update(data.as_bytes());
        let id = format!("{:x}", digest.finalize());
        let location = if file {
            let channels = iptv_core::parse_m3u(&data);
            if channels.is_empty() || channels.iter().any(|ch| !web_url(&ch.url)) {
                bail!("Choose an M3U playlist containing HTTP or HTTPS streams");
            }
            let files = dir.join("playlists");
            fs::create_dir_all(&files)?;
            let path = files.join(format!("{id}.m3u"));
            if !path.exists() {
                let tmp = path.with_extension("tmp");
                fs::write(&tmp, data)?;
                fs::rename(tmp, path)?;
            }
            id.clone()
        } else {
            data
        };
        self.entries.retain(|source| source.id != id);
        self.entries.push(Source {
            id: id.clone(),
            name,
            location,
            guide,
            file,
        });
        self.active = id;
        Ok(())
    }
}

impl Source {
    pub fn location(&self, dir: &Path) -> Result<String> {
        if self.file {
            if self.location.len() != 64 || !self.location.bytes().all(|c| c.is_ascii_hexdigit()) {
                bail!("Invalid playlist file");
            }
            return dir
                .join("playlists")
                .join(format!("{}.m3u", self.location))
                .into_os_string()
                .into_string()
                .ok()
                .context("Invalid playlist path");
        }
        Ok(self.location.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_uploaded_playlist_and_its_guide_after_switching_presets() {
        let dir = std::env::temp_dir().join(format!("iptv-sources-{}", std::process::id()));
        let mut catalog = Catalog::load(&dir);
        assert_eq!(catalog.current().location(&dir).unwrap(), DEFAULT);
        let guide = "https://example.test/guide.xml";
        catalog
            .select(
                Input::File {
                    name: "My channels".into(),
                    text: "#EXTM3U\n#EXTINF:-1,Channel\nhttps://example.test/live".into(),
                    guide: Some(guide.into()),
                },
                &dir,
            )
            .unwrap();
        let upload = catalog.active.clone();
        catalog
            .select(
                Input::Select {
                    id: "free-tv".into(),
                },
                &dir,
            )
            .unwrap();
        assert!(catalog.current().guide.is_none());
        catalog.save(&dir).unwrap();
        let mut catalog = Catalog::load(&dir);
        catalog.select(Input::Select { id: upload }, &dir).unwrap();
        assert_eq!(catalog.current().guide.as_deref(), Some(guide));
        assert!(
            fs::read_to_string(catalog.current().location(&dir).unwrap())
                .unwrap()
                .contains("#EXTM3U")
        );
        assert!(!catalog.public().to_string().contains("example.test"));
        fs::remove_dir_all(dir).unwrap();
    }
}
