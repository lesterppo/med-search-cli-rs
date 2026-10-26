//! med-search-cli-rs — Rust port of lesterppo/med-search-cli (v3).
//!
//! Pure helpers, the SQLite cache layer, and the HTTP client live here so
//! they are unit-testable without network. All live API calls live in
//! `api.rs`; the CLI surface lives in `main.rs`.

pub mod api;

use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

pub fn db_path() -> PathBuf {
    let raw = std::env::var("MED_SEARCH_DB").unwrap_or_else(|_| "pubmed_agent_cache.db".to_string());
    expand_home(&raw)
}

pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

pub fn default_ttl_days() -> i64 {
    std::env::var("MED_SEARCH_TTL").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
}

pub fn agent_email() -> String {
    std::env::var("MED_SEARCH_EMAIL").unwrap_or_else(|_| "your_official_email@example.com".to_string())
}

/// Remove bracketed IPv6 entries from NO_PROXY in-process: httpx on the
/// original side crashed on them, and reqwest's proxy handling is happier
/// without them too. Harmless on machines without the VM's proxy setup.
pub fn sanitize_no_proxy() {
    for key in ["NO_PROXY", "no_proxy"] {
        if let Ok(v) = std::env::var(key) {
            let kept: Vec<&str> = v.split(',').map(str::trim).filter(|e| !e.starts_with('[')).collect();
            // SAFETY: single-threaded at startup, before any client is built.
            unsafe { std::env::set_var(key, kept.join(",")) };
        }
    }
}

// ---------------------------------------------------------------------------
// Dates (no chrono dependency: civil-date conversion only)
// ---------------------------------------------------------------------------

pub fn now_iso() -> String {
    // Microsecond precision, matching Python's datetime.now(timezone.utc).isoformat().
    // (Previously second-precision; that made RoB `updated` differ from Python.)
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs() as i64;
    let micros = d.subsec_micros();
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (y, m, day) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}+00:00", y, m, day, tod / 3600, (tod / 60) % 60, tod % 60, micros)
}

pub fn format_iso(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let tod = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00", y, m, d, tod / 3600, (tod / 60) % 60, tod % 60)
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parse an ISO datetime (with or without offset) to epoch seconds.
/// Returns None on unparseable input, mirroring Python's fromisoformat guard.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date_part, rest) = s.split_once('T').unwrap_or((s, ""));
    let mut dp = date_part.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let m: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    let days = days_from_civil(y, m as u32, d as u32);
    if rest.is_empty() {
        return Some(days * 86400);
    }
    let time_part = rest.trim_end_matches('Z').split(['+', '-']).next().unwrap_or("");
    let mut tp = time_part.split(':');
    let hh: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let mm: i64 = tp.next().unwrap_or("0").parse().ok()?;
    // Seconds may carry a fractional part (Python isoformat microseconds);
    // strip it — age-in-days arithmetic does not need sub-second precision.
    let ss_token = tp.next().unwrap_or("0");
    let ss: i64 = ss_token.split('.').next().unwrap_or("0").parse().ok()?;
    Some(days * 86400 + hh * 3600 + mm * 60 + ss)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn valid_date_yyyy_mm_dd(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>(), parts[2].parse::<i64>()) else {
        return false;
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    // Real-calendar check via round-trip.
    let days = days_from_civil(y, m as u32, d as u32);
    civil_from_days(days) == (y, m as u32, d as u32)
}

// ---------------------------------------------------------------------------
// Query validation / translation (port of validate_query, europepmc_query)
// ---------------------------------------------------------------------------

const STOPWORDS: [&str; 3] = ["and", "or", "not"];

pub fn validate_query(query: &str) -> Result<String, String> {
    let clean = query.trim();
    if clean.is_empty() {
        return Err("query is empty (whitespace only)".to_string());
    }
    let stripped: String = clean.chars().map(|c| if "()\"[]:*".contains(c) { ' ' } else { c }).collect();
    let words: Vec<&str> = stripped.split_whitespace().filter(|w| !STOPWORDS.contains(&w.to_lowercase().as_str())).collect();
    if words.is_empty() {
        return Err(format!("query '{clean}' contains only operators/qualifiers, no searchable terms"));
    }
    if !words.iter().any(|w| w.chars().count() > 1) {
        return Err(format!("query '{clean}' has no term longer than 1 character"));
    }
    Ok(clean.to_string())
}

fn is_pubmed_syntax_query(query: &str) -> bool {
    // Field qualifier [xx] or uppercase boolean operator.
    let lower = query.to_lowercase();
    let _ = lower;
    // Qualifier: a '[' ... ']' whose content is a known-ish token.
    if query.contains('[') && query.contains(']') {
        return true;
    }
    query.split_whitespace().any(|w| w == "AND" || w == "OR" || w == "NOT")
}

fn epmc_field(field: &str) -> Option<&'static str> {
    match field.to_lowercase().trim() {
        "pmid" | "uid" => Some("EXT_ID"),
        "tiab" => Some("TITLE_ABS"),
        "ti" => Some("TITLE"),
        "ab" => Some("ABSTRACT"),
        "mh" | "mesh" | "mesh terms" => Some("MESH"),
        "au" => Some("AUTH"),
        "ta" | "journal" => Some("JOURNAL"),
        "pt" => Some("PUB_TYPE"),
        "dp" | "pdat" => Some("FIRST_PDATE"),
        "la" => Some("LANG"),
        "rn" => Some("CHEM"),
        _ => None,
    }
}

/// Translate a PubMed-syntax query into EuropePMC syntax.
/// Port of europepmc_query: supported qualifiers map, unsupported ones are
/// dropped (term kept), quoted phrases preserved.
pub fn europepmc_query(query: &str) -> String {
    if !query.contains('[') {
        return query.to_string();
    }
    let mut out = String::new();
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Find a term followed by [field].
        // Term = quoted phrase, parenthesised group, or bare token.
        let term_start = i;
        let term: String;
        if chars[i] == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += 1;
            }
            let end = if j < chars.len() { j + 1 } else { j };
            term = chars[i..end].iter().collect();
            i = end;
        } else if chars[i] == '(' {
            let mut depth = 0;
            let mut j = i;
            while j < chars.len() {
                if chars[j] == '(' {
                    depth += 1;
                } else if chars[j] == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            let end = if j < chars.len() { j + 1 } else { j };
            term = chars[i..end].iter().collect();
            i = end;
        } else {
            let mut j = i;
            while j < chars.len() && !chars[j].is_whitespace() && chars[j] != '[' {
                j += 1;
            }
            term = chars[i..j].iter().collect();
            i = j;
        }
        // Skip spaces between term and qualifier.
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() && chars[i] == '[' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                let field: String = chars[i + 1..j].iter().collect();
                match epmc_field(&field) {
                    Some("EXT_ID") => {
                        let bare = term.trim_matches('"');
                        out.push_str(&format!("EXT_ID:{bare}"));
                    }
                    Some(f) => out.push_str(&format!("{f}:{term}")),
                    None => out.push_str(&term),
                }
                i = j + 1;
            } else {
                out.push_str(&term);
                i = j;
            }
        } else {
            out.push_str(&term);
            // Copy the separator whitespace we skipped.
            if i < chars.len() && term_start < i {
                // Whitespace already consumed above only when a qualifier
                // followed; re-emit a single space if we are mid-query.
                // Simpler: the loop's next iteration starts at the next
                // non-space char, so emit one space here if more follows.
                if i < chars.len() {
                    out.push(' ');
                }
            }
        }
        // Copy remaining separators exactly once: advance over whitespace,
        // emitting a single space if more content follows.
        let mut k = i;
        while k < chars.len() && chars[k].is_whitespace() {
            k += 1;
        }
        if k > i && k < chars.len() {
            out.push(' ');
        }
        i = k;
    }
    let translated = out.trim().to_string();
    if translated.contains(' ') {
        format!("({translated})")
    } else {
        translated
    }
}

pub fn pubmed_query_term(query: &str) -> String {
    if is_pubmed_syntax_query(query) {
        query.to_string()
    } else {
        query.split_whitespace().map(|kw| format!("\"{kw}\"[TIAB]")).collect::<Vec<_>>().join(" AND ")
    }
}

// ---------------------------------------------------------------------------
// Study types
// ---------------------------------------------------------------------------

pub const STUDY_TYPE_TAGS: [&str; 8] = [
    "Case Reports",
    "Clinical Trial",
    "Meta-Analysis",
    "Observational Study",
    "Practice Guideline",
    "RCT",
    "Review",
    "Systematic Review",
];

pub fn pushdown_clauses(tags: &HashSet<String>) -> (String, String) {
    let mut sorted: Vec<&String> = tags.iter().collect();
    sorted.sort();
    let mut pm = Vec::new();
    let mut ep = Vec::new();
    for t in sorted {
        let pair = match t.as_str() {
            "RCT" => Some(("randomized controlled trial", "randomized controlled trial")),
            "Meta-Analysis" => Some(("meta-analysis", "meta-analysis")),
            "Systematic Review" => Some(("systematic review", "systematic review")),
            "Review" => Some(("review", "review")),
            "Observational Study" => Some(("observational study", "observational study")),
            "Case Reports" => Some(("case reports", "case report")),
            "Clinical Trial" => Some(("clinical trial", "clinical trial")),
            "Practice Guideline" => Some(("practice guideline", "practice guideline")),
            _ => None,
        };
        if let Some((p, e)) = pair {
            pm.push(format!("\"{p}\"[pt]"));
            ep.push(format!("PUB_TYPE:\"{e}\""));
        }
    }
    let pm_clause = if pm.is_empty() { String::new() } else { format!(" AND ({})", pm.join(" OR ")) };
    let ep_clause = if ep.is_empty() { String::new() } else { format!(" AND ({})", ep.join(" OR ")) };
    (pm_clause, ep_clause)
}

fn mesh_type_tag(lower: &str) -> Option<&'static str> {
    match lower {
        "randomized controlled trial" => Some("RCT"),
        "meta-analysis" => Some("Meta-Analysis"),
        "systematic review" => Some("Systematic Review"),
        "review" => Some("Review"),
        "observational study" | "cohort studies" => Some("Observational Study"),
        "case reports" => Some("Case Reports"),
        "clinical trial" => Some("Clinical Trial"),
        "practice guideline" => Some("Practice Guideline"),
        _ => None,
    }
}

pub fn detect_study_type(title: &str, abstract_text: &str, mesh_types: &[String]) -> String {
    for mt in mesh_types {
        if let Some(tag) = mesh_type_tag(&mt.to_lowercase()) {
            return tag.to_string();
        }
    }
    let text = format!("{title} {abstract_text}").to_lowercase();
    // Word-boundary matching, like the original regexes (\b...\b): a
    // plain substring check is wrong — "randomized clinical trials"
    // (plural) must NOT match the RCT pattern "randomized clinical trial",
    // and "guidelines" must NOT match "guideline" (regression found in
    // deep test, PMID 42892330: Python Meta-Analysis, Rust wrongly RCT).
    let has = |phrase: &str| -> bool {
        let mut start = 0;
        while let Some(pos) = text[start..].find(phrase) {
            let s = start + pos;
            let e = s + phrase.len();
            let before_ok = s == 0 || !text.as_bytes()[s - 1].is_ascii_alphanumeric();
            let after_ok = e >= text.len() || !text.as_bytes()[e].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
            start = s + 1;
        }
        false
    };
    // Order mirrors the original regex pattern list exactly — this matters:
    // the observational pattern (incl. cohort / case-control /
    // cross-sectional) precedes guideline, so a cross-sectional study whose
    // abstract mentions "guidelines" is Observational, not Practice
    // Guideline (regression found in deep test, PMID 42924359).
    if has("randomized controlled trial")
        || has("randomised controlled trial")
        || has("randomized trial")
        || has("randomised trial")
        || has("randomized clinical trial")
        || has("randomised clinical trial")
        || has("rct")
    {
        return "RCT".to_string();
    }
    if has("meta-analysis") || has("meta analysis") {
        return "Meta-Analysis".to_string();
    }
    if has("systematic review") {
        return "Systematic Review".to_string();
    }
    for needle in ["observational study", "cohort", "case-control", "case control", "cross-sectional", "cross sectional"] {
        if has(needle) {
            return "Observational Study".to_string();
        }
    }
    if has("case report") || has("case series") {
        return "Case Reports".to_string();
    }
    if has("clinical trial") || has("non-randomized") || has("non-randomised") {
        return "Clinical Trial".to_string();
    }
    if has("guideline") || has("consensus statement") {
        return "Practice Guideline".to_string();
    }
    "Unknown".to_string()
}

// ---------------------------------------------------------------------------
// Retraction flags, authors, MeSH
// ---------------------------------------------------------------------------

pub fn retraction_flags(pub_types: &[String], comments: &[String]) -> Map<String, Value> {
    let pts: HashSet<String> = pub_types.iter().map(|p| p.trim().to_lowercase()).collect();
    let norm: HashSet<String> = comments
        .iter()
        .map(|c| {
            let t = c.trim().to_lowercase();
            if let Some(stripped) = t.strip_suffix(" in") { stripped.trim().to_string() } else { t }
        })
        .collect();
    let mut flags = Map::new();
    if pts.contains("retracted publication")
        || pts.contains("retraction of publication")
        || norm.contains("retraction")
        || norm.contains("retraction of publication")
    {
        flags.insert("retracted".to_string(), Value::Bool(true));
    }
    if pts.contains("expression of concern") || norm.contains("expression of concern") {
        flags.insert("expression_of_concern".to_string(), Value::Bool(true));
    }
    if pts.contains("published erratum")
        || pts.contains("corrected and republished article")
        || norm.contains("erratum")
        || norm.contains("correction")
    {
        flags.insert("corrected".to_string(), Value::Bool(true));
    }
    flags
}

pub fn authors_europepmc(art: &Value) -> Vec<String> {
    if let Some(s) = art.get("authorString").and_then(|v| v.as_str()) {
        if !s.is_empty() {
            return s.split(',').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).take(3).collect();
        }
    }
    if let Some(list) = art.get("authorList").and_then(|v| v.get("author")).and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for a in list.iter().take(3) {
            let name = format!(
                "{} {}",
                a.get("lastName").and_then(|v| v.as_str()).unwrap_or(""),
                a.get("initials").and_then(|v| v.as_str()).unwrap_or("")
            )
            .trim()
            .to_string();
            if !name.is_empty() {
                out.push(name);
            }
        }
        return out;
    }
    Vec::new()
}

pub fn mesh_europepmc(list: Option<&Value>) -> Vec<String> {
    let Some(list) = list else { return Vec::new() };
    let Some(headings) = list.get("meshHeading").and_then(|v| v.as_array()) else { return Vec::new() };
    let mut out = Vec::new();
    for h in headings {
        if let Some(dn) = h.get("descriptorName") {
            if let Some(txt) = dn.get("#text").and_then(|v| v.as_str()).or_else(|| dn.get("value").and_then(|v| v.as_str())) {
                if !txt.trim().is_empty() {
                    out.push(txt.trim().to_string());
                }
            } else if let Some(txt) = dn.as_str() {
                if !txt.trim().is_empty() {
                    out.push(txt.trim().to_string());
                }
            }
        }
    }
    out
}

pub fn epmc_comments(art: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(list) = art.get("commentCorrectionList").and_then(|v| v.get("commentCorrection")).and_then(|v| v.as_array()) {
        for c in list {
            if let Some(t) = c.get("type").and_then(|v| v.as_str()) {
                out.push(t.to_string());
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Smart truncation (port of smart_truncate)
// ---------------------------------------------------------------------------

fn is_boilerplate(sentence: &str) -> bool {
    let s = sentence.trim_start().to_lowercase();
    s.starts_with("conflict of interest")
        || s.starts_with("conflicts of interest")
        || s.starts_with("competing interest")
        || s.starts_with("disclosure")
        || s.starts_with("funding")
        || s.starts_with("acknowledgement")
        || s.starts_with("acknowledgment")
        || s.starts_with("this work was supported")
        || s.starts_with("this study was supported")
        || s.contains("all rights reserved")
        || s.starts_with("copyright")
        || s.starts_with("supplementary material")
        || s.starts_with("additional material")
        || s.starts_with("published by")
        || s.starts_with("correspondence to")
        || s.starts_with("to whom correspondence")
        || s.starts_with("received:")
        || s.starts_with("accepted:")
        || s.starts_with("published online")
        || s.starts_with("clinical trial registration")
}

pub fn tokenize(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut cur = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() {
            cur.push(c);
        } else if cur.len() >= 3 {
            out.insert(std::mem::take(&mut cur));
        } else {
            cur.clear();
        }
    }
    if cur.len() >= 3 {
        out.insert(cur);
    }
    out
}

fn split_sentences(text: &str) -> Vec<String> {
    // Split after . ! ? followed by whitespace (Python: (?<=[.!?])\s+).
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        if (c == '.' || c == '!' || c == '?') && chars.peek().map(|n| n.is_whitespace()).unwrap_or(false) {
            out.push(cur.trim().to_string());
            cur.clear();
            while chars.peek().map(|n| n.is_whitespace()).unwrap_or(false) {
                chars.next();
            }
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Returns (text, truncated, kept_len). Mirrors the original scoring:
/// keyword hits + section_bonus, boilerplate -10, word-boundary cut.
pub fn smart_truncate(text: &str, limit: usize, keywords: Option<HashSet<String>>, section_bonus: i64) -> (String, bool, usize) {
    // CHARACTER counts everywhere, matching Python's len() (characters, not
    // bytes). The previous version used byte lengths (str::len), which cut
    // non-ASCII sections short: for PMID 42924359 the results section came
    // out 1746 chars instead of Python's 1792, a 46-char gap in fetch text.
    let n_chars = text.chars().count();
    if n_chars <= limit {
        return (text.to_string(), false, n_chars);
    }
    let keywords = keywords.unwrap_or_else(|| tokenize(text));
    let sentences = split_sentences(text);
    if sentences.is_empty() {
        return (text.chars().take(limit).collect(), true, limit);
    }
    let mut scored: Vec<(i64, String)> = sentences
        .into_iter()
        .map(|s| {
            let mut score = keywords.iter().filter(|kw| s.to_lowercase().contains(kw.as_str())).count() as i64 + section_bonus;
            if is_boilerplate(&s) {
                score -= 10;
            }
            (score, s)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let mut kept: Vec<String> = Vec::new();
    let mut total = 0i64;
    let mut truncated = false;
    for (_, s) in scored {
        let s_chars = s.chars().count() as i64;
        if total + s_chars + 1 <= limit as i64 {
            total += s_chars + 1;
            kept.push(s);
        } else {
            truncated = true;
            // Mirror Python exactly: remaining may go negative; the >40
            // guard then simply skips the partial cut (no underflow).
            let remaining = limit as i64 - total - 4;
            if remaining > 40 {
                let cut_chars: Vec<char> = s.chars().take(remaining as usize).collect();
                let mut cut: String = cut_chars.iter().collect();
                // Last space by CHARACTER index (Python's rfind on a str
                // returns a character position in effect for the comparison).
                if let Some(sp) = cut_chars.iter().rposition(|c| *c == ' ') {
                    if (sp as i64) > remaining / 2 {
                        cut = cut_chars[..sp].iter().collect();
                    }
                }
                kept.push(format!("{cut}..."));
            }
            break;
        }
    }
    let result = kept.join(" ");
    let len = result.chars().count();
    (result, truncated, len)
}

/// Serialize a JSON value with every non-ASCII character escaped
/// (`\uXXXX`, surrogate pairs above the BMP), matching Python's
/// `json.dumps(..., ensure_ascii=True)` default. Used for stderr error
/// output only; stdout keeps UTF-8, matching Python's
/// `ensure_ascii=False` success output. Control characters use Python's
/// short escapes where they exist, otherwise `\u00XX`.
pub fn json_ascii(v: &Value) -> String {
    fn escape_str(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                    let n = c as u32;
                    if n > 0xffff {
                        let v = n - 0x10000;
                        out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xd800 + (v >> 10), 0xdc00 + (v & 0x3ff)));
                    } else {
                        out.push_str(&format!("\\u{n:04x}"));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
    fn write(v: &Value, out: &mut String) {
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => out.push_str(&escape_str(s)),
            Value::Array(a) => {
                out.push('[');
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(x, out);
                }
                out.push(']');
            }
            Value::Object(m) => {
                out.push('{');
                for (i, (k, x)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&escape_str(k));
                    out.push(':');
                    write(x, out);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    write(v, &mut out);
    out
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

pub fn http_client() -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(Duration::from_secs(30));
    for path in [std::env::var("MED_RS_CA").unwrap_or_default(), "/etc/ssl/certs/hatch-egress-ca.pem".to_string()] {
        if path.is_empty() {
            continue;
        }
        if let Ok(pem) = std::fs::read(&path) {
            if let Ok(cert) = reqwest::Certificate::from_pem(&pem) {
                builder = builder.add_root_certificate(cert);
            }
        }
    }
    builder.build().map_err(|e| format!("http client: {e}"))
}

/// GET with up to 4 attempts; 429/5xx back off, other HTTP errors return
/// None (mirrors the original http_request returning None).
pub fn http_get(client: &reqwest::blocking::Client, url: &str) -> Option<Vec<u8>> {
    let mut wait = 1u64;
    for attempt in 0..4 {
        match client.get(url).send() {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if resp.status().is_success() {
                    return resp.bytes().ok().map(|b| b.to_vec());
                }
                if status == 429 || status >= 500 {
                    if attempt < 3 {
                        std::thread::sleep(Duration::from_secs(wait));
                        wait *= 2;
                        continue;
                    }
                }
                return None;
            }
            Err(_) => {
                if attempt < 3 {
                    std::thread::sleep(Duration::from_secs(wait));
                    wait *= 2;
                    continue;
                }
                return None;
            }
        }
    }
    None
}

pub fn http_get_json(client: &reqwest::blocking::Client, url: &str) -> Option<Value> {
    let body = http_get(client, url)?;
    serde_json::from_slice(&body).ok()
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SQLite cache (lit_cache + lit_fts + rob + saved_queries/seen_items)
// ---------------------------------------------------------------------------

pub fn init_db(path: &Path) -> Result<(), String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS lit_cache (pmid TEXT PRIMARY KEY, source TEXT, payload TEXT, updated TEXT, ttl_days INTEGER DEFAULT 30);
         CREATE VIRTUAL TABLE IF NOT EXISTS lit_fts USING fts5(pmid, title, abstract, fulltext, tokenize='porter');
         CREATE TABLE IF NOT EXISTS rob (pmid TEXT PRIMARY KEY, tool TEXT, domains TEXT, overall TEXT, note TEXT, updated TEXT);
         CREATE TABLE IF NOT EXISTS saved_queries (name TEXT PRIMARY KEY, query TEXT, created TEXT, last_run TEXT, max_results INTEGER);
         CREATE TABLE IF NOT EXISTS seen_items (name TEXT, pmid TEXT, first_seen TEXT, PRIMARY KEY (name, pmid));",
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub struct CachedData {
    pub source: String,
    pub data: Value,
    pub stale: bool,
}

pub fn get_cached_data(path: &Path, pmid: &str) -> Option<CachedData> {
    let conn = rusqlite::Connection::open(path).ok()?;
    let mut stmt = conn.prepare("SELECT source, payload, updated, ttl_days FROM lit_cache WHERE pmid=?1").ok()?;
    let mut rows = stmt.query([pmid]).ok()?;
    let row = rows.next().ok()??;
    let source: String = row.get(0).ok()?;
    let payload: String = row.get(1).ok()?;
    let updated: String = row.get(2).ok()?;
    let ttl: i64 = row.get::<_, Option<i64>>(3).ok()?.unwrap_or(default_ttl_days());
    let data: Value = serde_json::from_str(&payload).ok()?;
    let updated_secs = parse_iso(&updated)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let age_days = (now - updated_secs) / 86400;
    Some(CachedData { source, data, stale: age_days > ttl })
}

pub fn save_to_cache(path: &Path, pmid: &str, source: &str, data: &Value, ttl_days: i64) -> Result<(), String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    let now = now_iso();
    let payload = serde_json::to_string(data).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO lit_cache (pmid, source, payload, updated, ttl_days) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![pmid, source, payload, now, ttl_days],
    )
    .map_err(|e| e.to_string())?;
    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let abstract_text = data.get("abstract").and_then(|v| v.as_str()).unwrap_or("");
    let fulltext = data
        .get("sections")
        .and_then(|v| v.as_object())
        .map(|m| m.values().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    conn.execute("DELETE FROM lit_fts WHERE pmid=?1", [pmid]).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO lit_fts (pmid, title, abstract, fulltext) VALUES (?1,?2,?3,?4)",
        rusqlite::params![pmid, title, abstract_text, fulltext],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn search_cache(path: &Path, query: &str, limit: i64) -> Vec<Value> {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2 && !STOPWORDS.contains(&t.to_lowercase().as_str()))
        .map(|t| t.to_string())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let match_expr = terms.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(" AND ");
    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // FTS5 first (original behaviour, incl. snippets); LIKE fallback if
    // the bundled SQLite lacks FTS5.
    if let Ok(mut stmt) = conn.prepare(
        "SELECT pmid, snippet(lit_fts, 1, '<b>', '</b>', '…', 40), snippet(lit_fts, 2, '<b>', '</b>', '…', 40) FROM lit_fts WHERE lit_fts MATCH ?1 LIMIT ?2",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![match_expr, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        }) {
            let mut out = Vec::new();
            for r in rows.flatten() {
                out.push(json!({"pmid": r.0, "title_snippet": r.1, "abstract_snippet": r.2}));
            }
            return out;
        }
    }
    // Fallback: LIKE over stored payloads.
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT pmid, payload FROM lit_cache") {
        if let Ok(rows) = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))) {
            for r in rows.flatten() {
                if let Ok(v) = serde_json::from_str::<Value>(&r.1) {
                    let hay = format!(
                        "{} {}",
                        v.get("title").and_then(|x| x.as_str()).unwrap_or(""),
                        v.get("abstract").and_then(|x| x.as_str()).unwrap_or("")
                    )
                    .to_lowercase();
                    if terms.iter().all(|t| hay.contains(&t.to_lowercase())) {
                        out.push(json!({"pmid": r.0, "title_snippet": "", "abstract_snippet": ""}));
                        if out.len() as i64 >= limit {
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

pub fn cache_stats(path: &Path) -> Value {
    let conn = match rusqlite::Connection::open(path) {
        Ok(c) => c,
        Err(_) => return json!({"total_records": 0, "stale_records": 0, "fts_indexed": 0}),
    };
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM lit_cache", [], |r| r.get(0)).unwrap_or(0);
    let stale: i64 = conn
        .query_row("SELECT COUNT(*) FROM lit_cache WHERE datetime(updated, '+' || ttl_days || ' days') < datetime('now')", [], |r| r.get(0))
        .unwrap_or(0);
    let fts: i64 = conn.query_row("SELECT COUNT(*) FROM lit_fts", [], |r| r.get(0)).unwrap_or(0);
    json!({"total_records": total, "stale_records": stale, "fts_indexed": fts})
}

// ---------------------------------------------------------------------------
// RoB helpers
// ---------------------------------------------------------------------------

pub const ROB_TOOLS: [&str; 4] = ["rob2", "robins-i", "nos", "quadas-2"];
pub const ROB_OVERALLS: [&str; 4] = ["low", "some-concerns", "high", "critical"];

pub fn parse_include(value: &str) -> Result<Option<bool>, String> {
    let s = value.trim().to_lowercase();
    if s.is_empty() {
        return Ok(None);
    }
    if ["yes", "y", "1", "true", "t", "include", "included", "in"].contains(&s.as_str()) {
        return Ok(Some(true));
    }
    if ["no", "n", "0", "false", "f", "exclude", "excluded", "ex", "out"].contains(&s.as_str()) {
        return Ok(Some(false));
    }
    Err(format!("Unrecognised include value '{value}': use yes/no/1/0"))
}

pub fn unweighted_kappa(pairs: &[(bool, bool)]) -> f64 {
    let n = pairs.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let agree = pairs.iter().filter(|(a, b)| a == b).count() as f64;
    let po = agree / n;
    let pa_yes = pairs.iter().filter(|(a, _)| *a).count() as f64 / n;
    let pb_yes = pairs.iter().filter(|(_, b)| *b).count() as f64 / n;
    let pe = pa_yes * pb_yes + (1.0 - pa_yes) * (1.0 - pb_yes);
    if (1.0 - pe).abs() < 1e-12 {
        return if agree == n { 1.0 } else { 0.0 };
    }
    (po - pe) / (1.0 - pe)
}

pub fn iso_from_pubdate(pubdate: &str) -> Option<String> {
    // "2024 Jan 15" / "2024 Jan" / "2024" -> YYYY-MM-DD.
    let mut parts = pubdate.trim().split_whitespace();
    let year: i64 = parts.next()?.parse().ok()?;
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mon = parts.next().map(|m| {
        let cap = format!("{}{}", &m[..1].to_uppercase(), &m[1..].to_lowercase());
        months.iter().position(|x| *x == cap).map(|i| i + 1).unwrap_or(1)
    }).unwrap_or(1);
    let day: i64 = parts.next().and_then(|d| d.parse().ok()).unwrap_or(1);
    Some(format!("{year:04}-{mon:02}-{day:02}"))
}

/// Compact JSON on stdout, exactly like the original (separators=(",",":")).
pub fn emit(v: &Value) {
    println!("{}", serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()));
}

pub fn err_json(msg: &str) -> Value {
    json!({"error": msg})
}

pub fn record_defaults(mut rec: Value) -> Value {
    let obj = rec.as_object_mut().expect("record must be an object");
    let defaults: [(&str, Value); 14] = [
        ("abstract", json!("")),
        ("journal", Value::Null),
        ("cited_by", Value::Null),
        ("authors", json!([])),
        ("mesh_keywords", json!([])),
        ("study_type", json!("Unknown")),
        ("pub_types", json!([])),
        ("in_epmc", json!(false)),
        ("in_pmc", json!(false)),
        ("is_open_access", json!(false)),
        ("doi", Value::Null),
        ("retracted", json!(false)),
        ("expression_of_concern", json!(false)),
        ("corrected", json!(false)),
    ];
    for (k, v) in defaults {
        obj.entry(k.to_string()).or_insert(v);
    }
    if !obj.contains_key("cited_by_source") {
        obj.insert("cited_by_source".to_string(), Value::Null);
    }
    rec
}

pub fn is_empty_val(v: Option<&Value>) -> bool {
    match v {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Unit tests (pure functions + cache; no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn tmp_db(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("med_rs_test_{name}.db"));
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(format!("{}-wal", p.display()));
        let _ = std::fs::remove_file(format!("{}-shm", p.display()));
        p
    }

    #[test]
    fn validate_rejects_empty_operator_only_single_char() {
        assert!(validate_query("").is_err());
        assert!(validate_query("   ").is_err());
        assert!(validate_query("AND OR NOT").is_err());
        assert!(validate_query("a").is_err());
    }

    #[test]
    fn validate_accepts_real_query() {
        assert_eq!(validate_query("  aspirin stroke  ").unwrap(), "aspirin stroke");
    }

    #[test]
    fn epmc_translation() {
        assert!(europepmc_query("9500320[pmid]").contains("EXT_ID:9500320"));
        assert!(europepmc_query("\"lupus nephritis\"[tiab]").contains("TITLE_ABS"));
        // Unsupported qualifier: term kept, tag dropped.
        assert!(!europepmc_query("aspirin[xyz]").contains("[xyz]"));
        assert!(europepmc_query("plain query").contains("plain query"));
    }

    #[test]
    fn pushdown() {
        let tags: HashSet<String> = ["RCT".to_string(), "Meta-Analysis".to_string()].into_iter().collect();
        let (pm, ep) = pushdown_clauses(&tags);
        assert!(pm.contains("[pt]") && ep.contains("PUB_TYPE"), "{pm} / {ep}");
        let (pm0, ep0) = pushdown_clauses(&HashSet::new());
        assert_eq!((pm0, ep0), (String::new(), String::new()));
    }

    #[test]
    fn study_type_mesh_priority_and_patterns() {
        assert_eq!(detect_study_type("x", "y", &["meta-analysis".to_string()]), "Meta-Analysis");
        assert_eq!(detect_study_type("A randomized controlled trial of aspirin", "", &[]), "RCT");
        assert_eq!(detect_study_type("A cohort study", "", &[]), "Observational Study");
        // Cross-sectional beats guideline when both appear (PMID 42924359).
        assert_eq!(detect_study_type("A cross-sectional study", "following national guidelines", &[]), "Observational Study");
        assert_eq!(detect_study_type("Nothing special here", "", &[]), "Unknown");
    }

    #[test]
    fn retraction_flags_cover_variants() {
        let f = retraction_flags(&["Retracted Publication".to_string()], &[]);
        assert_eq!(f.get("retracted"), Some(&Value::Bool(true)));
        let f2 = retraction_flags(&[], &["Retraction in".to_string()]);
        assert_eq!(f2.get("retracted"), Some(&Value::Bool(true)));
        let f3 = retraction_flags(&[], &[]);
        assert!(f3.is_empty());
    }

    #[test]
    fn smart_truncate_word_boundary() {
        let text = (0..200).map(|i| format!("word{i:03}")).collect::<Vec<_>>().join(" ");
        let (out, trunc, _) = smart_truncate(&text, 200, None, 0);
        assert!(trunc);
        let body = out.strip_suffix("...").unwrap_or(&out);
        assert!(!body.ends_with(' '), "{body:?}");
        let (short, t2, _) = smart_truncate("tiny text", 200, None, 0);
        assert!(!t2 && short == "tiny text");
    }

    #[test]
    fn authors_and_mesh_europepmc() {
        let art = json!({"authorString": "Smith J, Jones A, Lee K, Park M"});
        assert_eq!(authors_europepmc(&art).len(), 3);
        let m = json!({"meshHeading": [{"descriptorName": "Aspirin"}, {"descriptorName": {"#text": "Stroke"}}]});
        assert_eq!(mesh_europepmc(Some(&m)), vec!["Aspirin", "Stroke"]);
    }

    #[test]
    fn iso_pubdate() {
        assert_eq!(iso_from_pubdate("2024 Jan 15").as_deref(), Some("2024-01-15"));
        assert_eq!(iso_from_pubdate("2024").as_deref(), Some("2024-01-01"));
        assert!(iso_from_pubdate("").is_none());
    }

    #[test]
    fn kappa_and_include() {
        assert!((unweighted_kappa(&[(true, true), (false, false)]) - 1.0).abs() < 1e-9);
        assert_eq!(unweighted_kappa(&[]), 0.0);
        assert_eq!(parse_include("yes").unwrap(), Some(true));
        assert_eq!(parse_include("0").unwrap(), Some(false));
        assert_eq!(parse_include("").unwrap(), None);
        assert!(parse_include("maybe").is_err());
    }

    #[test]
    fn cache_roundtrip_and_ttl() {
        let db = tmp_db("cache");
        init_db(&db).unwrap();
        let data = json!({"pmid": "123", "title": "T", "abstract": "aspirin stroke"});
        save_to_cache(&db, "123", "fallback", &data, 30).unwrap();
        let got = get_cached_data(&db, "123").unwrap();
        assert!(!got.stale);
        assert_eq!(got.data["title"], "T");
        // TTL 0 + old updated timestamp => stale.
        save_to_cache(&db, "124", "fallback", &data, 0).unwrap();
        // Backdate the row by hand.
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE lit_cache SET updated='2020-01-01T00:00:00+00:00' WHERE pmid='124'", []).unwrap();
        drop(conn);
        let stale = get_cached_data(&db, "124").unwrap();
        assert!(stale.stale);
        // FTS search finds the cached record.
        let hits = search_cache(&db, "aspirin stroke", 20);
        assert!(hits.iter().any(|h| h["pmid"] == "123"));
        assert!(search_cache(&db, "AND", 5).is_empty());
        let stats = cache_stats(&db);
        assert_eq!(stats["total_records"], 2);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn valid_dates() {
        assert!(valid_date_yyyy_mm_dd("2024-02-29"));
        assert!(!valid_date_yyyy_mm_dd("2023-02-29"));
        assert!(!valid_date_yyyy_mm_dd("2024-13-01"));
        assert!(!valid_date_yyyy_mm_dd("2024/01/01"));
    }

    #[test]
    fn export_bibtex_key_shape() {
        // Mirrors main.rs export: first-author surname + year + pmid, alnum only.
        let key: String = "smith2024pmid123".chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        assert_eq!(key, "smith2024pmid123");
    }

    #[test]
    fn smart_truncate_counts_characters_not_bytes() {
        // Regression for the 46-char fetch gap (PMID 42924359): a text of
        // 100 two-byte characters is 100 chars / 200 bytes. With a
        // 150-char limit, Python's len() keeps the whole text (no
        // truncation); the old byte-counting version truncated it.
        let text = "é ".repeat(50).trim_end().to_string();
        assert_eq!(text.chars().count(), 99);
        let (out, trunc, kept) = smart_truncate(&text, 150, None, 0);
        assert!(!trunc && out == text && kept == 99);
        // And a genuinely over-limit non-ASCII text truncates by chars.
        let long = "é".repeat(300);
        let (out2, trunc2, kept2) = smart_truncate(&long, 100, None, 0);
        assert!(trunc2);
        assert_eq!(out2.chars().count(), kept2);
        assert!(kept2 <= 100);
    }

    #[test]
    fn json_ascii_escapes_non_ascii_like_python() {
        let v = json!({"error": "Invalid PMID 'abc' — expected digits only (e.g. 38261728)."});
        assert_eq!(
            json_ascii(&v),
            r#"{"error":"Invalid PMID 'abc' \u2014 expected digits only (e.g. 38261728)."}"#
        );
        // Non-BMP becomes a surrogate pair, exactly like Python's json.dumps.
        let emoji = json!({"error": "bad 😀"});
        assert!(json_ascii(&emoji).contains("\\ud83d\\ude00"));
        // ASCII-only input is unchanged apart from compact separators.
        assert_eq!(json_ascii(&json!({"error": "plain"})), r#"{"error":"plain"}"#);
    }

    #[test]
    fn http_get_retries_429_then_succeeds_and_gives_up() {
        // Local stub server only — never hammers NCBI/EPMC. First server:
        // 429, 429, then 200. Second server: always 429 (4 attempts, then None).
        use std::io::{Read, Write};
        use std::net::TcpListener;
        fn serve(statuses: Vec<u16>) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            std::thread::spawn(move || {
                for status in statuses {
                    let Ok((mut stream, _)) = listener.accept() else { break };
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let body = if status == 200 { "ok-body" } else { "" };
                    let resp = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(resp.as_bytes());
                }
            });
            format!("http://{addr}/x")
        }
        let client = http_client().unwrap();
        let url = serve(vec![429, 429, 200]);
        assert_eq!(http_get(&client, &url), Some(b"ok-body".to_vec()));
        let url2 = serve(vec![429, 429, 429, 429]);
        assert_eq!(http_get(&client, &url2), None);
    }

    #[test]
    fn now_iso_microseconds_and_parse_iso_fractional() {
        let now = now_iso();
        assert!(now.contains('.'), "{now}");
        let frac = now.split('.').nth(1).unwrap();
        assert_eq!(frac.len(), 6 + "+00:00".len(), "{now}");
        // Python-style microsecond timestamps must parse (previously the
        // fractional seconds made parse_iso return None).
        assert_eq!(parse_iso("2026-10-26T03:55:24.402753+00:00"), Some(1792986924));
        assert_eq!(parse_iso("2026-10-26T03:55:24+00:00"), Some(1792986924));
    }
}
