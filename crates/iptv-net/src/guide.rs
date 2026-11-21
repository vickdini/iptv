//! XMLTV program information shared by the desktop and Android players.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Result};
use chrono::NaiveDateTime;
use serde::Deserialize;

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Default)]
pub struct Guide(HashMap<String, Vec<Program>>);

struct Program {
    start: i64,
    stop: i64,
    title: String,
}

#[derive(Deserialize)]
#[serde(rename = "tv")]
struct XmlTv {
    #[serde(rename = "programme", default)]
    programs: Vec<XmlProgram>,
}

#[derive(Deserialize)]
struct XmlProgram {
    #[serde(rename = "@channel", default)]
    channel: String,
    #[serde(rename = "@start", default)]
    start: String,
    #[serde(rename = "@stop", default)]
    stop: String,
    #[serde(default)]
    title: Vec<XmlTitle>,
}

#[derive(Deserialize)]
struct XmlTitle {
    #[serde(rename = "$text", default)]
    text: String,
}

fn timestamp(value: &str) -> Option<i64> {
    let mut fields = value.split_whitespace();
    let date = fields.next()?;
    match fields.next() {
        Some(offset) if offset != "UTC" => {
            chrono::DateTime::parse_from_str(&format!("{date} {offset}"), "%Y%m%d%H%M%S %z")
                .ok()
                .map(|date| date.timestamp())
        }
        _ => NaiveDateTime::parse_from_str(date, "%Y%m%d%H%M%S")
            .ok()
            .map(|date| date.and_utc().timestamp()),
    }
}

impl Guide {
    pub fn load(source: &str) -> Result<Self> {
        use std::io::BufRead;
        let reader: Box<dyn Read> =
            if source.starts_with("http://") || source.starts_with("https://") {
                ureq::AgentBuilder::new()
                    .timeout(std::time::Duration::from_secs(30))
                    .build()
                    .get(source)
                    .call()?
                    .into_reader()
            } else {
                Box::new(File::open(source)?)
            };
        let mut reader = BufReader::new(reader);
        if reader.fill_buf()?.starts_with(&[0x1f, 0x8b]) {
            Self::read(flate2::read::GzDecoder::new(reader))
        } else {
            Self::read(reader)
        }
    }

    fn read(reader: impl Read) -> Result<Self> {
        let mut xml = String::new();
        reader.take(MAX_BYTES + 1).read_to_string(&mut xml)?;
        if xml.len() as u64 > MAX_BYTES {
            bail!("XMLTV guide exceeds 32 MiB");
        }
        let mut root = quick_xml::Reader::from_str(&xml);
        loop {
            match root.read_event()? {
                quick_xml::events::Event::Start(tag) | quick_xml::events::Event::Empty(tag)
                    if tag.name().as_ref() == b"tv" =>
                {
                    break
                }
                quick_xml::events::Event::Start(_)
                | quick_xml::events::Event::Empty(_)
                | quick_xml::events::Event::Eof => bail!("Expected an XMLTV document"),
                _ => {}
            }
        }
        let feed: XmlTv = quick_xml::de::from_str(&xml)?;
        let mut guide = Self::default();
        for entry in feed.programs {
            if entry.channel.is_empty() {
                continue;
            }
            let (Some(start), Some(title)) = (
                timestamp(&entry.start),
                entry
                    .title
                    .into_iter()
                    .map(|title| title.text)
                    .find(|title| !title.trim().is_empty()),
            ) else {
                continue;
            };
            guide.0.entry(entry.channel).or_default().push(Program {
                start,
                stop: timestamp(&entry.stop).unwrap_or(start),
                title: title.split_whitespace().collect::<Vec<_>>().join(" "),
            });
        }
        for programs in guide.0.values_mut() {
            programs.sort_by_key(|program| program.start);
            for i in 0..programs.len().saturating_sub(1) {
                if programs[i].stop <= programs[i].start {
                    programs[i].stop = programs[i + 1].start;
                }
            }
        }
        Ok(guide)
    }

    pub fn now_next(&self, channel: &iptv_core::Channel) -> (Option<&str>, Option<&str>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.at(channel.tvg_id.as_deref().unwrap_or(""), now)
    }

    fn at(&self, id: &str, now: i64) -> (Option<&str>, Option<&str>) {
        let Some(programs) = self
            .0
            .get(id)
            .or_else(|| self.0.get(iptv_core::m3u::station_id(id)))
        else {
            return (None, None);
        };
        let next = programs.partition_point(|program| program.start <= now);
        let current = next
            .checked_sub(1)
            .and_then(|i| programs.get(i))
            .filter(|program| now < program.stop);
        (
            current.map(|program| program.title.as_str()),
            programs.get(next).map(|p| p.title.as_str()),
        )
    }

    pub fn labels(&self, channel: &iptv_core::Channel) -> [Option<String>; 2] {
        let (now, next) = self.now_next(channel);
        [
            now.map(|title| iptv_core::i18n::tf("Now: {0}", &[&title])),
            next.map(|title| iptv_core::i18n::tf("Next: {0}", &[&title])),
        ]
    }

    pub fn summary(&self, channel: &iptv_core::Channel, separator: &str) -> String {
        self.labels(channel)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(separator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_variants_and_handles_gaps_and_missing_channels() {
        let xml = br#"<tv>
          <programme channel="example" start="20260101010000 +0100" stop="20260101013000 +0100"><title>News &amp; Weather</title></programme>
          <programme channel="example" start="20260101010000 +0000"><title>Film</title></programme>
        </tv>"#;
        let guide = Guide::read(&xml[..]).unwrap();
        let start = timestamp("20260101000000 +0000").unwrap();
        assert_eq!(
            guide.at("example@HD", start),
            (Some("News & Weather"), Some("Film"))
        );
        assert_eq!(guide.at("example", start + 1800), (None, Some("Film")));
        assert_eq!(guide.at("other", start), (None, None));
    }
}
