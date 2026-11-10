//! Readable names for the language codes found in streams' audio and
//! subtitle tracks (ISO 639-1 / 639-2, plus the codes broadcasters use for
//! "original version" and "audio description").

const TABLE: &[(&[&str], &str)] = &[
    (&["", "und", "unk", "mul", "zxx"], "Unknown"),
    (&["qaa", "ori", "orig", "original", "mis"], "Original"),
    (&["qad", "aud", "ad"], "Audio description"),
    (&["en", "eng"], "English"),
    (&["es", "spa"], "Spanish"),
    (&["fr", "fra", "fre"], "French"),
    (&["de", "deu", "ger"], "German"),
    (&["it", "ita"], "Italian"),
    (&["pt", "por"], "Portuguese"),
    (&["nl", "nld", "dut"], "Dutch"),
    (&["ca", "cat"], "Catalan"),
    (&["eu", "eus", "baq"], "Basque"),
    (&["gl", "glg"], "Galician"),
    (&["ru", "rus"], "Russian"),
    (&["uk", "ukr"], "Ukrainian"),
    (&["pl", "pol"], "Polish"),
    (&["cs", "ces", "cze"], "Czech"),
    (&["sk", "slk", "slo"], "Slovak"),
    (&["hu", "hun"], "Hungarian"),
    (&["ro", "ron", "rum"], "Romanian"),
    (&["bg", "bul"], "Bulgarian"),
    (&["el", "ell", "gre"], "Greek"),
    (&["tr", "tur"], "Turkish"),
    (&["ar", "ara"], "Arabic"),
    (&["he", "heb"], "Hebrew"),
    (&["fa", "fas", "per"], "Persian"),
    (&["hi", "hin"], "Hindi"),
    (&["bn", "ben"], "Bengali"),
    (&["ur", "urd"], "Urdu"),
    (&["ta", "tam"], "Tamil"),
    (&["te", "tel"], "Telugu"),
    (&["zh", "zho", "chi"], "Chinese"),
    (&["ja", "jpn"], "Japanese"),
    (&["ko", "kor"], "Korean"),
    (&["vi", "vie"], "Vietnamese"),
    (&["th", "tha"], "Thai"),
    (&["id", "ind"], "Indonesian"),
    (&["ms", "msa", "may"], "Malay"),
    (&["sv", "swe"], "Swedish"),
    (&["no", "nor", "nb", "nob", "nn", "nno"], "Norwegian"),
    (&["da", "dan"], "Danish"),
    (&["fi", "fin"], "Finnish"),
    (&["sr", "srp"], "Serbian"),
    (&["hr", "hrv"], "Croatian"),
    (&["sl", "slv"], "Slovenian"),
    (&["sq", "sqi", "alb"], "Albanian"),
    (&["ka", "kat", "geo"], "Georgian"),
    (&["hy", "hye", "arm"], "Armenian"),
    (&["az", "aze"], "Azerbaijani"),
    (&["kk", "kaz"], "Kazakh"),
];

fn entry(code: &str) -> Option<&'static (&'static [&'static str], &'static str)> {
    let c = code.trim().to_ascii_lowercase();
    // "en-US" / "pt_BR": the language part is enough.
    let base = c.split(['-', '_']).next().unwrap_or("");
    TABLE.iter().find(|(codes, _)| codes.contains(&base))
}

/// "en" / "eng" -> "English", "qad" -> "Audio description". Unknown codes are
/// returned upper-cased; empty means "Unknown".
pub fn language_name(code: &str) -> String {
    match entry(code) {
        Some((_, name)) => name.to_string(),
        None => code.trim().to_ascii_uppercase(),
    }
}

/// Whether two track language codes mean the same language ("es" / "spa").
pub fn same_language(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.eq_ignore_ascii_case(b) {
        return true;
    }
    match (entry(a), entry(b)) {
        (Some(x), Some(y)) => std::ptr::eq(x, y) && x.1 != "Unknown",
        _ => false,
    }
}

/// Every code for the language of `code` (for players that take a list of
/// preferred languages), the given code first.
pub fn language_codes(code: &str) -> Vec<String> {
    let first = code.trim().to_string();
    let mut out = vec![first.clone()];
    if let Some((codes, name)) = entry(code) {
        if *name != "Unknown" {
            out.extend(
                codes
                    .iter()
                    .filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case(&first))
                    .map(|c| c.to_string()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_codes() {
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("ITA"), "Italian");
        assert_eq!(language_name("qaa"), "Original");
        assert_eq!(language_name("qad"), "Audio description");
        assert_eq!(language_name(""), "Unknown");
        assert_eq!(language_name("xyz"), "XYZ");
        assert_eq!(language_name("pt-BR"), "Portuguese");
    }

    #[test]
    fn matches_codes() {
        assert!(same_language("es", "spa"));
        assert!(same_language("FRE", "fr"));
        assert!(!same_language("es", "en"));
        assert!(!same_language("und", ""));
        assert_eq!(language_codes("spa"), ["spa", "es"]);
    }
}
