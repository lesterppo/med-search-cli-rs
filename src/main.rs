//! med-search-rs CLI — clap surface over med_search_cli_rs + api.
//! Output contract mirrors the Python original: compact JSON on stdout,
//! validation errors as compact JSON on stderr with exit 2, upstream
//! errors on stderr with exit 1 (fetch single-PMID resolution failure).

use clap::{Parser, Subcommand};
use med_search_cli_rs::api::*;
use med_search_cli_rs::*;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

fn err_exit(msg: &str, code: i32) -> ! {
    eprintln!("{}", serde_json::to_string(&json!({"error": msg})).unwrap());
    std::process::exit(code);
}

fn err_exit_val(v: Value, code: i32) -> ! {
    eprintln!("{}", serde_json::to_string(&v).unwrap());
    std::process::exit(code);
}

fn require_pmid(pmid: &str) -> String {
    let clean = pmid.trim();
    if clean.len() > 12 || clean.is_empty() || !clean.chars().all(|c| c.is_ascii_digit()) {
        err_exit(&format!("Invalid PMID '{pmid}' — expected digits only (e.g. 38261728)."), 2);
    }
    clean.to_string()
}

fn study_set(single: Option<String>, multi: Option<String>) -> Result<HashSet<String>, String> {
    let mut set = HashSet::new();
    if let Some(s) = single {
        set.insert(s);
    }
    if let Some(m) = multi {
        for t in m.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
            if !STUDY_TYPE_TAGS.contains(&t) {
                return Err(format!("Unknown study type(s): {t}. Valid: {}", STUDY_TYPE_TAGS.join(", ")));
            }
            set.insert(t.to_string());
        }
    }
    Ok(set)
}

#[derive(Parser)]
#[command(name = "med-search-rs", version, about = "PubMed + EuropePMC biomedical literature search (Rust port of med-search-cli)")]
struct Cli {
    #[command(subcommand)]
    cmd: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search PubMed + EuropePMC in parallel, deduplicate, enrich, filter
    Search {
        #[arg(short = 'q', long)]
        query: String,
        #[arg(short = 'm', long, default_value_t = 5)]
        max_results: i64,
        #[arg(short = 'f', long)]
        from_date: Option<String>,
        #[arg(short = 't', long)]
        to_date: Option<String>,
        #[arg(short = 'S', long, default_value = "relevance", value_parser = ["relevance", "date", "citations"])]
        sort: String,
        #[arg(short = 'C', long, default_value_t = 0)]
        min_citations: i64,
        #[arg(short = 'T', long, value_parser = STUDY_TYPE_TAGS)]
        study_type: Option<String>,
        #[arg(short = 'U', long)]
        study_types: Option<String>,
        #[arg(short = 'V', long)]
        verbose: bool,
    },
    /// Fetch one or more papers by PMID
    Fetch {
        #[arg(short = 'p', long)]
        pmid: String,
        #[arg(short = 's', long, default_value = "all", value_parser = ["all", "abstract", "intro", "methods", "results", "discussion"])]
        section: String,
        #[arg(short = 'l', long, default_value_t = 6000)]
        limit: i64,
        #[arg(long, default_value_t = 30)]
        ttl: i64,
    },
    /// Show cache statistics
    #[command(name = "cache-stats")]
    CacheStats,
    /// Full-text search across locally cached papers (zero network calls)
    #[command(name = "search-cache")]
    SearchCache {
        #[arg(short = 'q', long)]
        query: String,
        #[arg(short = 'l', long, default_value_t = 20)]
        limit: i64,
    },
    /// Look up MeSH descriptors for a term
    Mesh {
        term: String,
        #[arg(short = 'l', long, default_value_t = 5)]
        limit: i64,
        #[arg(long)]
        exact: bool,
        #[arg(long)]
        counts: bool,
    },
    /// Similar articles (PubMed neighbor links)
    Related {
        #[arg(short = 'p', long)]
        pmid: String,
        #[arg(short = 'm', long, default_value_t = 10)]
        max_results: i64,
    },
    /// Who cites this paper
    Citedby {
        #[arg(short = 'p', long)]
        pmid: String,
        #[arg(short = 'm', long, default_value_t = 10)]
        max_results: i64,
        #[arg(short = 'S', long, default_value = "date", value_parser = ["date", "citations"])]
        sort: String,
    },
    /// Reference list of a paper
    Refs {
        #[arg(short = 'p', long)]
        pmid: String,
        #[arg(short = 'm', long, default_value_t = 15)]
        max_results: i64,
    },
    /// Export records for a reference manager
    Export {
        #[arg(short = 'p', long)]
        pmids: String,
        #[arg(short = 'F', long, default_value = "bibtex", value_parser = ["bibtex", "ris", "csv", "json"])]
        format: String,
        #[arg(short = 'o', long)]
        out: Option<String>,
        #[arg(long)]
        screen: bool,
    },
    /// Agreement stats for a dual-reviewed screening CSV
    Reconcile {
        csv_path: Option<String>,
        #[arg(short = 'c', long)]
        csv: Option<String>,
        #[arg(short = 'F', long, default_value = "json", value_parser = ["json", "text"])]
        format: String,
    },
    /// Store or fetch risk-of-bias judgements
    Rob {
        #[arg(short = 'p', long)]
        pmid: Option<String>,
        #[arg(long)]
        tool: Option<String>,
        #[arg(short = 'd', long = "domain")]
        domains: Vec<String>,
        #[arg(long)]
        overall: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(short = 'l', long = "list")]
        list_only: bool,
        #[arg(short = 'F', long, default_value = "json", value_parser = ["json", "text"])]
        format: String,
    },
    /// ClinicalTrials.gov search
    Trials {
        #[arg(short = 'q', long)]
        query: String,
        #[arg(short = 'm', long, default_value_t = 5)]
        max_results: i64,
        #[arg(short = 's', long)]
        status: Option<String>,
        #[arg(long)]
        full: bool,
    },
    /// Standing-query surveillance: report only papers not seen before
    Watch {
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(short = 'q', long)]
        query: Option<String>,
        #[arg(short = 'm', long)]
        max_results: Option<i64>,
        #[arg(short = 'L', long = "list")]
        list_only: bool,
        #[arg(long)]
        forget: bool,
    },
    /// PRISMA-style flow counts from existing local state only
    Prisma {
        #[arg(short = 'n', long)]
        name: Option<String>,
        #[arg(long, default_value_t = 0)]
        screened: i64,
        #[arg(long, default_value_t = 0)]
        eligible: i64,
        #[arg(long, default_value_t = 0)]
        included: i64,
        #[arg(short = 'F', long, default_value = "json", value_parser = ["json", "text"])]
        format: String,
    },
}

fn validate_dates(from_date: Option<&str>, to_date: Option<&str>) {
    for (label, val) in [("from_date", from_date), ("to_date", to_date)] {
        if let Some(v) = val {
            if !valid_date_yyyy_mm_dd(v) {
                // Distinguish shape vs real-date errors as the original does.
                let shape = v.len() == 10 && v.chars().nth(4) == Some('-') && v.chars().nth(7) == Some('-');
                if shape {
                    err_exit(&format!("Invalid {label}: '{v}' is not a real calendar date. Use YYYY-MM-DD."), 2);
                } else {
                    err_exit(&format!("Invalid {label}: '{v}'. Use YYYY-MM-DD."), 2);
                }
            }
        }
    }
    if let (Some(f), Some(t)) = (from_date, to_date) {
        if f > t {
            err_exit(&format!("from_date {f} is after to_date {t}"), 2);
        }
    }
}

fn interleave(records: Vec<Value>) -> Vec<Value> {
    let both: Vec<Value> = records.iter().filter(|r| r["source"].as_str() == Some("both")).cloned().collect();
    let pm: Vec<Value> = records.iter().filter(|r| r["source"].as_str() == Some("pubmed")).cloned().collect();
    let ep: Vec<Value> = records.iter().filter(|r| r["source"].as_str() == Some("europepmc")).cloned().collect();
    let mut out = both;
    for (i, p) in pm.iter().enumerate() {
        out.push(p.clone());
        if let Some(e) = ep.get(i) {
            out.push(e.clone());
        }
    }
    if ep.len() > pm.len() {
        out.extend(ep[pm.len()..].iter().cloned());
    }
    out
}

fn run_search(client: &reqwest::blocking::Client, query: &str, max_results: usize, from_date: Option<&str>, to_date: Option<&str>, sort: &str, min_citations: i64, allowed: &HashSet<String>) -> (Vec<Value>, Vec<Value>, usize) {
    let (pm_clause, ep_clause) = pushdown_clauses(allowed);
    let pool = (max_results * 3).max(20);
    let (us, eu) = twin_search(client, query, pool, from_date, to_date, sort, &pm_clause, &ep_clause, min_citations);
    (us, eu, pool)
}

fn search_core(client: &reqwest::blocking::Client, query: &str, max_results: usize, from_date: Option<&str>, to_date: Option<&str>, sort: &str, min_citations: i64, allowed: &HashSet<String>) -> (Vec<Value>, usize, usize, usize, usize) {
    let (us, eu, _) = run_search(client, query, max_results, from_date, to_date, sort, min_citations, allowed);
    let merged = merge_records(us.clone(), eu.clone());
    let pre_filter = merged.len();
    let mut final_records = interleave(merged);
    if !allowed.is_empty() {
        final_records.retain(|r| allowed.contains(r["study_type"].as_str().unwrap_or("Unknown")));
    }
    let mut unknown = 0usize;
    if min_citations > 0 {
        let mut kept = Vec::new();
        for r in final_records {
            match r.get("cited_by").and_then(|v| v.as_i64()) {
                None => unknown += 1,
                Some(cb) if cb >= min_citations => kept.push(r),
                _ => {}
            }
        }
        final_records = kept;
    }
    final_records.truncate(max_results);
    if sort == "citations" {
        final_records.sort_by(|a, b| {
            let an = a.get("cited_by").and_then(|v| v.as_i64()).is_none();
            let bn = b.get("cited_by").and_then(|v| v.as_i64()).is_none();
            an.cmp(&bn).then_with(|| {
                let av = a.get("cited_by").and_then(|v| v.as_i64()).unwrap_or(0);
                let bv = b.get("cited_by").and_then(|v| v.as_i64()).unwrap_or(0);
                bv.cmp(&av)
            })
        });
    }
    for r in &mut final_records {
        let flags: Vec<&str> = ["retracted", "expression_of_concern", "corrected"].into_iter().filter(|f| r.get(*f).and_then(|v| v.as_bool()) == Some(true)).collect();
        if !flags.is_empty() {
            r["warning"] = Value::String(if r.get("retracted").and_then(|v| v.as_bool()) == Some(true) { "RETRACTED".to_string() } else { flags[0].to_uppercase() });
        }
    }
    (final_records, us.len(), eu.len(), pre_filter, unknown)
}

// ---------------------------------------------------------------------------
// fetch
// ---------------------------------------------------------------------------

fn fetch_one(client: &reqwest::blocking::Client, db: &PathBuf, pmid: &str, section: &str, limit: i64, ttl: i64) -> Value {
    let cached = if ttl > 0 { get_cached_data(db, pmid) } else { None };
    let (mut out, source): (Value, String) = if let Some(c) = cached {
        if !c.stale {
            let mut d = c.data.clone();
            if d.get("date").and_then(|v| v.as_str()).is_none() {
                if let Some(meta) = pmid_metadata(client, pmid) {
                    d["date"] = meta["date"].clone();
                    if d.get("commentary").is_none() {
                        d["commentary"] = meta["commentary"].clone();
                    }
                    let _ = save_to_cache(db, pmid, &c.source, &d, ttl);
                }
            }
            d["cached"] = Value::Bool(true);
            (d, c.source)
        } else {
            let mut d = c.data.clone();
            d["cached_stale"] = Value::Bool(true);
            (d, c.source)
        }
    } else {
        let Some(meta) = pmid_metadata(client, pmid) else {
            return json!({"error": format!("No PubMed record found for PMID {pmid} (invalid or deleted)."), "pmid": pmid});
        };
        let doi = meta.get("doi").and_then(|v| v.as_str()).map(|s| s.to_string());
        let mut d = json!({
            "pmid": pmid,
            "doi": meta["doi"],
            "title": meta["title"],
            "abstract": meta["abstract"],
            "date": meta["date"],
            "study_type": meta.get("study_type").cloned().unwrap_or(json!("Unknown")),
            "commentary": meta.get("commentary").cloned().unwrap_or(json!(false)),
            "sections": Value::Null,
            "open_access": Value::Null,
            "proxy_url": Value::Null,
            "journal": meta["journal"],
            "authors": meta["authors"],
            "mesh_keywords": meta["mesh_keywords"],
            "pub_types": meta["pub_types"],
        });
        for flag in ["retracted", "expression_of_concern", "corrected"] {
            if meta.get(flag).and_then(|v| v.as_bool()) == Some(true) {
                d[flag] = Value::Bool(true);
            }
        }
        let mut src = "fallback".to_string();
        // Sequential full-text attempts (original: parallel, first finisher
        // wins; sequential PubMed-first is deterministic — see README).
        let mut sections: Option<Map<String, Value>> = None;
        if let Some(s) = fetch_pubmed_fulltext(client, pmid) {
            let body_len: usize = s.values().filter_map(|v| v.as_str()).map(|x| x.len()).sum();
            if body_len >= 200 {
                sections = Some(s);
                src = "pubmed_ft".to_string();
            }
        }
        if sections.is_none() {
            if let Some(s) = fetch_europepmc_fulltext(client, pmid) {
                let body_len: usize = s.values().filter_map(|v| v.as_str()).map(|x| x.len()).sum();
                if body_len >= 200 {
                    sections = Some(s);
                    src = "europepmc_ft".to_string();
                }
            }
        }
        if let Some(s) = sections {
            d["sections"] = Value::Object(s);
        }
        if src == "fallback" {
            if let Some(doi_s) = &doi {
                if let Some(oa) = fetch_unpaywall(client, doi_s) {
                    d["open_access"] = oa;
                    src = "unpaywall".to_string();
                }
            }
        }
        if let Some(doi_s) = &doi {
            if let Ok(prefix) = std::env::var("INSTITUTIONAL_PROXY_PREFIX") {
                if !prefix.is_empty() {
                    d["proxy_url"] = Value::String(format!("{prefix}{}", url_encode(&format!("https://doi.org/{doi_s}"))));
                }
            }
        }
        let _ = save_to_cache(db, pmid, &src, &d, ttl);
        (d, src)
    };

    let abstract_text = out.get("abstract").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let mut resp = json!({
        "pmid": pmid,
        "title": out["title"],
        "date": out.get("date").cloned().unwrap_or(Value::Null),
        "source": source,
        "section": section,
        "study_type": out.get("study_type").cloned().unwrap_or(json!("Unknown")),
    });
    if out.get("retracted").and_then(|v| v.as_bool()) == Some(true) {
        resp["warning"] = Value::String("RETRACTED".to_string());
    } else if out.get("expression_of_concern").and_then(|v| v.as_bool()) == Some(true) {
        resp["warning"] = Value::String("EXPRESSION_OF_CONCERN".to_string());
    } else if out.get("corrected").and_then(|v| v.as_bool()) == Some(true) {
        resp["warning"] = Value::String("CORRECTED".to_string());
    }
    if out.get("commentary").and_then(|v| v.as_bool()) == Some(true) {
        resp["note"] = Value::String("Editorial/letter/commentary — no abstract available".to_string());
    }
    for opt in ["doi", "journal", "authors", "mesh_keywords", "proxy_url", "pub_types", "cached", "cached_stale"] {
        if let Some(v) = out.get(opt) {
            if !is_empty_val(Some(v)) && *v != Value::Bool(false) {
                resp[opt] = v.clone();
            }
        }
    }
    let sections_map: Option<Map<String, Value>> = out.get("sections").and_then(|v| v.as_object()).cloned();
    let mut raw = String::new();
    let mut used_abstract = false;
    let mut section_bonus = 0i64;
    if section == "abstract" {
        raw = abstract_text.clone();
    } else if section == "all" {
        if let Some(secs) = &sections_map {
            let alloc: [(&str, f64); 5] = [("discussion", 0.35), ("results", 0.30), ("methods", 0.20), ("intro", 0.10), ("other", 0.05)];
            let bonus: HashMap<&str, i64> = [("discussion", 2), ("results", 1), ("methods", 0), ("intro", 0), ("other", 0)].into_iter().collect();
            let keywords = tokenize(&format!("{} {}", out.get("title").and_then(|v| v.as_str()).unwrap_or(""), abstract_text));
            let mut parts: Vec<String> = Vec::new();
            for (sec_name, frac) in alloc {
                if let Some(sec_text) = secs.get(sec_name).and_then(|v| v.as_str()) {
                    if sec_text.trim().is_empty() {
                        continue;
                    }
                    let alloc_limit = (limit as f64 * frac) as usize;
                    let alloc_limit = alloc_limit.max(200);
                    let (trimmed, _, _) = smart_truncate(sec_text, alloc_limit, Some(keywords.clone()), *bonus.get(sec_name).unwrap_or(&0));
                    if !trimmed.trim().is_empty() {
                        parts.push(trimmed);
                    }
                }
            }
            raw = parts.join("\n\n");
        }
        if raw.trim().is_empty() {
            raw = abstract_text.clone();
            used_abstract = true;
            if sections_map.is_none() {
                if let Some(oa) = out.get("open_access").and_then(|v| v.get("pdf")) {
                    resp["pdf"] = oa.clone();
                }
            }
        }
    } else {
        if let Some(secs) = &sections_map {
            if let Some(sec_text) = secs.get(section).and_then(|v| v.as_str()) {
                if !sec_text.trim().is_empty() {
                    section_bonus = if section == "discussion" { 2 } else if section == "results" { 1 } else { 0 };
                    raw = sec_text.to_string();
                } else {
                    raw = abstract_text.clone();
                    used_abstract = true;
                    resp["note"] = Value::String(format!("Section '{section}' not available{}", if abstract_text.is_empty() { "" } else { " — returning abstract instead" }));
                    if let Some(oa) = out.get("open_access").and_then(|v| v.get("pdf")) {
                        resp["pdf"] = oa.clone();
                    }
                }
            } else {
                raw = abstract_text.clone();
                used_abstract = true;
                resp["note"] = Value::String(format!("Section '{section}' not available{}", if abstract_text.is_empty() { "" } else { " — returning abstract instead" }));
                if let Some(oa) = out.get("open_access").and_then(|v| v.get("pdf")) {
                    resp["pdf"] = oa.clone();
                }
            }
        } else {
            raw = abstract_text.clone();
            used_abstract = true;
            resp["note"] = Value::String(format!("Section '{section}' not available{}", if abstract_text.is_empty() { "" } else { " — returning abstract instead" }));
            if let Some(oa) = out.get("open_access").and_then(|v| v.get("pdf")) {
                resp["pdf"] = oa.clone();
            }
        }
    }
    if !raw.is_empty() {
        if raw.len() as i64 > limit {
            let keywords = tokenize(&format!("{} {}", out.get("title").and_then(|v| v.as_str()).unwrap_or(""), abstract_text));
            let (trimmed, was_truncated, kept) = smart_truncate(&raw, limit as usize, Some(keywords), section_bonus);
            resp["text"] = Value::String(trimmed);
            resp["truncated"] = Value::Bool(was_truncated);
            if was_truncated {
                resp["truncated_bytes"] = Value::Number(kept.into());
            }
        } else {
            resp["text"] = Value::String(raw);
        }
        if used_abstract {
            resp["text_from"] = Value::String("abstract".to_string());
            if resp.get("note").is_none() {
                resp["note"] = Value::String("Abstract only — no open-access full text found".to_string());
            }
        }
    } else if resp.get("note").is_none() {
        resp["note"] = Value::String("No text available for this article (no abstract, no OA full text)".to_string());
    }
    resp
}

// ---------------------------------------------------------------------------
// CSV helpers (reconcile + export)
// ---------------------------------------------------------------------------

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            in_quotes = true;
        } else if c == ',' {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else if c == '\r' {
            // Skip; \n follows.
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn rob_lookup(db: &PathBuf, pmids: &[String]) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    if pmids.is_empty() {
        return out;
    }
    let Ok(conn) = rusqlite::Connection::open(db) else { return out };
    let placeholders = pmids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT pmid, tool, overall FROM rob WHERE pmid IN ({placeholders})");
    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::types::ToSql> = pmids.iter().map(|p| p as &dyn rusqlite::types::ToSql).collect();
        if let Ok(rows) = stmt.query_map(&params[..], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))) {
            for r in rows.flatten() {
                out.insert(r.0, (r.1.unwrap_or_default(), r.2.unwrap_or_default()));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    sanitize_no_proxy();
    let cli = Cli::parse();
    let db = db_path();
    if let Err(e) = init_db(&db) {
        err_exit(&format!("cache init failed: {e}"), 1);
    }
    let client = match http_client() {
        Ok(c) => c,
        Err(e) => err_exit(&e, 1),
    };

    match cli.cmd {
        Commands::Search { query, max_results, from_date, to_date, sort, min_citations, study_type, study_types, verbose } => {
            let clean = match validate_query(&query) {
                Ok(c) => c,
                Err(e) => err_exit_val(json!({"error": e, "query": query}), 2),
            };
            validate_dates(from_date.as_deref(), to_date.as_deref());
            if let (Some(f), Some(t)) = (&from_date, &to_date) {
                if f > t {
                    err_exit(&format!("from_date {f} is after to_date {t}"), 2);
                }
            }
            if max_results < 1 {
                err_exit(&format!("Invalid --max-results {max_results}: must be >= 1."), 2);
            }
            if min_citations < 0 {
                err_exit(&format!("Invalid --min-citations {min_citations}: must be >= 0."), 2);
            }
            let allowed = match study_set(study_type, study_types) {
                Ok(s) => s,
                Err(e) => err_exit(&e, 2),
            };
            let (final_records, n_us, n_eu, pre_filter, unknown) = search_core(&client, &clean, max_results as usize, from_date.as_deref(), to_date.as_deref(), &sort, min_citations, &allowed);
            emit(&Value::Array(final_records.clone()));
            if verbose {
                eprintln!("{}", serde_json::to_string(&json!({"pool": {"pubmed": n_us, "europepmc": n_eu, "deduped": pre_filter}, "kept": final_records.len(), "dropped_no_citation_data": unknown, "retracted_in_results": final_records.iter().filter(|r| r.get("retracted").and_then(|v| v.as_bool()) == Some(true)).count()})).unwrap());
            }
        }
        Commands::Fetch { pmid, section, limit, ttl } => {
            if limit <= 0 {
                err_exit(&format!("Invalid --limit {limit}: must be a positive character budget."), 2);
            }
            if ttl < 0 {
                err_exit(&format!("Invalid --ttl {ttl}: must be >= 0 days (0 bypasses cache)."), 2);
            }
            let pmids: Vec<String> = pmid.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
            if pmids.is_empty() {
                err_exit("No valid PMIDs provided.", 2);
            }
            let bad: Vec<String> = pmids.iter().filter(|p| p.len() > 12 || !p.chars().all(|c| c.is_ascii_digit())).cloned().collect();
            if !bad.is_empty() {
                err_exit(&format!("Invalid PMID(s): {} — expected digits only.", bad.join(", ")), 2);
            }
            if pmids.len() > 50 {
                err_exit(&format!("{} PMIDs requested — limit is 50 per call.", pmids.len()), 2);
            }
            if pmids.len() == 1 {
                let result = fetch_one(&client, &db, &pmids[0], &section, limit, ttl);
                if result.get("error").is_some() {
                    err_exit_val(result, 1);
                }
                emit(&result);
            } else {
                let results: Vec<Value> = pmids.iter().map(|p| fetch_one(&client, &db, p, &section, limit, ttl)).collect();
                emit(&Value::Array(results));
            }
        }
        Commands::CacheStats => {
            emit(&cache_stats(&db));
        }
        Commands::SearchCache { query, limit } => {
            if limit < 1 {
                err_exit(&format!("Invalid --limit {limit}: must be >= 1."), 2);
            }
            if let Err(e) = validate_query(&query) {
                err_exit_val(json!({"error": e, "query": query}), 2);
            }
            emit(&Value::Array(search_cache(&db, &query, limit)));
        }
        Commands::Mesh { term, limit, exact, counts } => {
            let clean = match validate_query(&term) {
                Ok(c) => c,
                Err(e) => err_exit_val(json!({"error": e}), 2),
            };
            if limit < 1 {
                err_exit(&format!("Invalid --limit {limit}: must be >= 1."), 2);
            }
            match mesh_lookup(&client, &clean, limit, exact, counts) {
                Ok(out) => {
                    let n = out.len();
                    emit(&json!({"term": term, "n": n, "r": out}));
                }
                Err(e) => err_exit(&e, 1),
            }
        }
        Commands::Related { pmid, max_results } => {
            let pid = require_pmid(&pmid);
            if max_results < 1 {
                err_exit(&format!("Invalid --max-results {max_results}: must be >= 1."), 2);
            }
            let records = related(&client, &pid, max_results as usize);
            if records.is_empty() {
                emit(&json!({"pmid": pid, "n": 0, "r": [], "note": "No related articles returned."}));
            } else {
                let n = records.len();
                emit(&json!({"pmid": pid, "n": n, "r": records}));
            }
        }
        Commands::Citedby { pmid, max_results, sort } => {
            let pid = require_pmid(&pmid);
            if max_results < 1 {
                err_exit(&format!("Invalid --max-results {max_results}: must be >= 1."), 2);
            }
            match citedby(&client, &pid, max_results as usize) {
                None => err_exit_val(json!({"error": "EuropePMC citations request failed.", "pmid": pid}), 1),
                Some((hit_count, mut out)) => {
                    if sort == "date" {
                        out.sort_by(|a, b| b.get("year").and_then(|v| v.as_str()).unwrap_or("").cmp(a.get("year").and_then(|v| v.as_str()).unwrap_or("")));
                    }
                    let n = out.len();
                    emit(&json!({"pmid": pid, "hit_count": hit_count, "n": n, "r": out}));
                }
            }
        }
        Commands::Refs { pmid, max_results } => {
            let pid = require_pmid(&pmid);
            if max_results < 1 {
                err_exit(&format!("Invalid --max-results {max_results}: must be >= 1."), 2);
            }
            let (hit_count, out, note) = refs(&client, &pid, max_results as usize);
            if out.is_empty() {
                let mut v = json!({"pmid": pid, "n": 0, "r": [], "note": format!("{} Use `citedby` (forward) or `related` instead.", note.clone().unwrap_or_else(|| "No reference list available".to_string()))});
                let _ = &mut v;
                emit(&v);
            } else {
                let n = out.len();
                let mut v = json!({"pmid": pid, "hit_count": hit_count, "n": n, "r": out});
                if let Some(note_s) = note {
                    v["note"] = Value::String(note_s);
                }
                emit(&v);
            }
        }
        Commands::Export { pmids, format, out, screen } => {
            let ids: Vec<String> = pmids.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).map(|p| require_pmid(&p)).collect();
            if ids.is_empty() {
                err_exit("no PMIDs given", 2);
            }
            if screen && format != "csv" {
                err_exit("--screen applies to csv only; rerun with -F csv", 2);
            }
            let summ = esummary_batch(&client, &ids);
            let mut records: Vec<Value> = Vec::new();
            for pid in &ids {
                if let Some(rec) = summ.get(pid) {
                    let mut r = esummary_to_record(pid, rec);
                    let meta = pmid_metadata(&client, pid).unwrap_or(json!({}));
                    r["abstract"] = meta.get("abstract").cloned().unwrap_or(json!(""));
                    r["journal_full"] = meta.get("journal").cloned().unwrap_or_else(|| r["journal"].clone());
                    r["authors_full"] = meta.get("authors").cloned().unwrap_or_else(|| r["authors"].clone());
                    r["mesh"] = meta.get("mesh_keywords").cloned().unwrap_or(json!([]));
                    if meta.get("retracted").and_then(|v| v.as_bool()) == Some(true) {
                        r["warning"] = Value::String("RETRACTED".to_string());
                    }
                    records.push(r);
                } else {
                    records.push(json!({"pmid": pid, "error": "not found"}));
                }
            }
            let text = match format.as_str() {
                "json" => serde_json::to_string(&records).unwrap(),
                "csv" => {
                    let screen_fields: Vec<&str> = if screen { vec!["include_a", "reason_a", "include_b", "reason_b", "consensus", "notes", "rob_tool", "rob_overall"] } else { vec![] };
                    let mut fields: Vec<String> = ["pmid", "title", "journal", "date", "study_type", "doi", "warning", "authors"].iter().map(|s| s.to_string()).collect();
                    fields.extend(screen_fields.iter().map(|s| s.to_string()));
                    let mut buf = String::new();
                    buf.push_str(&fields.iter().map(|f| csv_escape(f)).collect::<Vec<_>>().join(","));
                    buf.push('\n');
                    let rob_map = if screen { rob_lookup(&db, &ids) } else { HashMap::new() };
                    for r in &records {
                        let mut row: Vec<String> = Vec::new();
                        for f in &fields {
                            let val = match f.as_str() {
                                "authors" => r.get("authors").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("; ")).unwrap_or_default(),
                                "rob_tool" => rob_map.get(r.get("pmid").and_then(|v| v.as_str()).unwrap_or("")).map(|(t, _)| t.clone()).unwrap_or_default(),
                                "rob_overall" => rob_map.get(r.get("pmid").and_then(|v| v.as_str()).unwrap_or("")).map(|(_, o)| o.clone()).unwrap_or_default(),
                                _ => r.get(f).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            };
                            row.push(csv_escape(&val));
                        }
                        buf.push_str(&row.join(","));
                        buf.push('\n');
                    }
                    buf
                }
                "ris" => {
                    let mut lines: Vec<String> = Vec::new();
                    for r in &records {
                        lines.push("TY  - JOUR".to_string());
                        if let Some(authors) = r.get("authors").and_then(|v| v.as_array()) {
                            for a in authors {
                                if let Some(s) = a.as_str() {
                                    lines.push(format!("AU  - {s}"));
                                }
                            }
                        }
                        lines.push(format!("TI  - {}", r.get("title").and_then(|v| v.as_str()).unwrap_or("")));
                        lines.push(format!("JO  - {}", r.get("journal_full").and_then(|v| v.as_str()).or_else(|| r.get("journal").and_then(|v| v.as_str())).unwrap_or("")));
                        let date = r.get("date").and_then(|v| v.as_str()).unwrap_or("");
                        lines.push(format!("PY  - {}", &date[..date.len().min(4)]));
                        lines.push(format!("DA  - {date}"));
                        if let Some(doi) = r.get("doi").and_then(|v| v.as_str()) {
                            if !doi.is_empty() {
                                lines.push(format!("DO  - {doi}"));
                            }
                        }
                        lines.push(format!("AN  - {}", r.get("pmid").and_then(|v| v.as_str()).unwrap_or("")));
                        lines.push(format!("UR  - https://pubmed.ncbi.nlm.nih.gov/{}/", r.get("pmid").and_then(|v| v.as_str()).unwrap_or("")));
                        lines.push("ER  - ".to_string());
                        lines.push(String::new());
                    }
                    lines.join("\n")
                }
                _ => {
                    // bibtex
                    let mut lines: Vec<String> = Vec::new();
                    for r in &records {
                        let pmid_s = r.get("pmid").and_then(|v| v.as_str()).unwrap_or("");
                        let first_author = r.get("authors").and_then(|v| v.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()).unwrap_or("Unknown");
                        let auth = first_author.split_whitespace().next().unwrap_or("Unknown").to_lowercase();
                        let date = r.get("date").and_then(|v| v.as_str()).unwrap_or("");
                        let year = if date.len() >= 4 { &date[..4] } else { "n.d." };
                        let key: String = format!("{auth}{year}pmid{pmid_s}").chars().filter(|c| c.is_ascii_alphanumeric()).collect();
                        lines.push(format!("@article{{{key},"));
                        lines.push(format!("  pmid = {{{pmid_s}}},"));
                        lines.push(format!("  title = {{{}}},", r.get("title").and_then(|v| v.as_str()).unwrap_or("")));
                        if let Some(authors_full) = r.get("authors_full").and_then(|v| v.as_array()) {
                            if !authors_full.is_empty() {
                                let joined = authors_full.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" and ");
                                lines.push(format!("  author = {{{joined}}},"));
                            }
                        }
                        lines.push(format!("  journal = {{{}}},", r.get("journal_full").and_then(|v| v.as_str()).or_else(|| r.get("journal").and_then(|v| v.as_str())).unwrap_or("")));
                        lines.push(format!("  year = {{{year}}},"));
                        if let Some(doi) = r.get("doi").and_then(|v| v.as_str()) {
                            if !doi.is_empty() {
                                lines.push(format!("  doi = {{{doi}}},"));
                            }
                        }
                        lines.push(format!("  url = {{https://pubmed.ncbi.nlm.nih.gov/{pmid_s}/}}"));
                        lines.push("}".to_string());
                        lines.push(String::new());
                    }
                    lines.join("\n")
                }
            };
            if let Some(out_path) = out {
                let expanded = expand_home(&out_path);
                match std::fs::write(&expanded, &text) {
                    Ok(_) => emit(&json!({"wrote": expanded.display().to_string(), "format": format, "n": records.len(), "screen": screen})),
                    Err(e) => err_exit(&e.to_string(), 1),
                }
            } else {
                println!("{text}");
            }
        }
        Commands::Reconcile { csv_path, csv, format } => {
            let path = (csv.clone().or(csv_path).unwrap_or_default()).trim().to_string();
            if path.is_empty() {
                err_exit("no CSV given: reconcile FILE or reconcile --csv FILE", 2);
            }
            let expanded = expand_home(&path);
            if !expanded.is_file() {
                err_exit(&format!("CSV not found: {}", expanded.display()), 2);
            }
            let text = match std::fs::read_to_string(&expanded) {
                Ok(t) => t,
                Err(e) => err_exit(&e.to_string(), 1),
            };
            let rows = parse_csv(&text);
            if rows.is_empty() {
                err_exit(&format!("CSV has no header: {}", expanded.display()), 2);
            }
            let headers: Vec<String> = rows[0].iter().map(|h| h.trim().to_lowercase()).collect();
            let col = |name: &str| headers.iter().position(|h| h == name);
            let (Some(c_pmid), Some(c_a), Some(c_b)) = (col("pmid"), col("include_a"), col("include_b")) else {
                let missing: Vec<&str> = ["pmid", "include_a", "include_b"].into_iter().filter(|n| col(n).is_none()).collect();
                err_exit(&format!("CSV missing columns: {}", missing.join(", ")), 2);
            };
            let mut pairs: Vec<(bool, bool)> = Vec::new();
            let mut disagreements: Vec<String> = Vec::new();
            let mut skipped = 0usize;
            for (i, row) in rows.iter().skip(1).enumerate() {
                let pmid = row.get(c_pmid).map(|s| s.trim().to_string()).unwrap_or_default();
                let a = match parse_include(row.get(c_a).map(|s| s.as_str()).unwrap_or("")) {
                    Ok(v) => v,
                    Err(e) => err_exit(&format!("row {} (pmid {}): {e}", i + 2, if pmid.is_empty() { "?" } else { &pmid }), 2),
                };
                let b = match parse_include(row.get(c_b).map(|s| s.as_str()).unwrap_or("")) {
                    Ok(v) => v,
                    Err(e) => err_exit(&format!("row {} (pmid {}): {e}", i + 2, if pmid.is_empty() { "?" } else { &pmid }), 2),
                };
                match (a, b) {
                    (Some(aa), Some(bb)) => {
                        pairs.push((aa, bb));
                        if aa != bb && !pmid.is_empty() {
                            disagreements.push(pmid);
                        }
                    }
                    _ => skipped += 1,
                }
            }
            let n = pairs.len();
            let agree = pairs.iter().filter(|(a, b)| a == b).count();
            let agree_pct = if n > 0 { (agree as f64 / n as f64 * 100.0 * 10.0).round() / 10.0 } else { 0.0 };
            let kappa = if n > 0 { (unweighted_kappa(&pairs) * 10000.0).round() / 10000.0 } else { 0.0 };
            if format == "text" {
                println!("Reconciliation: n={n} judged, agree={agree}, disagree={} ({agree_pct}%), kappa={kappa}, skipped={skipped}", n - agree);
                println!("Disagreements: {}", if disagreements.is_empty() { "none".to_string() } else { disagreements.join(", ") });
            } else {
                emit(&json!({"n": n, "agree": agree, "disagree": n - agree, "agree_pct": agree_pct, "kappa": kappa, "disagreements": disagreements, "skipped": skipped}));
            }
        }
        Commands::Rob { pmid, tool, domains, overall, note, list_only, format } => {
            let conn = match rusqlite::Connection::open(&db) {
                Ok(c) => c,
                Err(e) => err_exit(&e.to_string(), 1),
            };
            let emit_rob = |obj: Value| {
                if format == "text" {
                    if let Some(arr) = obj.as_array() {
                        if arr.is_empty() {
                            println!("No risk-of-bias judgements stored.");
                        }
                        for r in arr {
                            println!("{} [{}] {} {}{}", r["pmid"].as_str().unwrap_or(""), r["tool"].as_str().unwrap_or(""), r["overall"].as_str().unwrap_or(""), r["domains"], if r["note"].as_str().unwrap_or("").is_empty() { String::new() } else { format!(" — {}", r["note"].as_str().unwrap_or("")) });
                        }
                    } else {
                        println!("{} [{}] {} {}{}", obj["pmid"].as_str().unwrap_or(""), obj["tool"].as_str().unwrap_or(""), obj["overall"].as_str().unwrap_or(""), obj["domains"], if obj["note"].as_str().unwrap_or("").is_empty() { String::new() } else { format!(" — {}", obj["note"].as_str().unwrap_or("")) });
                    }
                } else {
                    emit(&obj);
                }
            };
            if list_only {
                let mut out = Vec::new();
                if let Ok(mut stmt) = conn.prepare("SELECT pmid, tool, domains, overall, note, updated FROM rob ORDER BY pmid") {
                    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?))) {
                        for r in rows.flatten() {
                            let doms: Value = r.2.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(json!({}));
                            out.push(json!({"pmid": r.0, "tool": r.1, "domains": doms, "overall": r.3, "note": r.4.unwrap_or_default(), "updated": r.5.unwrap_or_default()}));
                        }
                    }
                }
                if format == "text" {
                    emit_rob(Value::Array(out));
                } else {
                    let n = out.len();
                    emit_rob(json!({"n": n, "r": out}));
                }
                return;
            }
            let Some(pmid_s) = pmid else { err_exit("rob needs -p PMID or --list", 2) };
            let clean = require_pmid(&pmid_s);
            let wants_write = tool.is_some() || overall.is_some() || !domains.is_empty() || note.is_some();
            if !wants_write {
                let row = conn.query_row("SELECT pmid, tool, domains, overall, note, updated FROM rob WHERE pmid=?1", [&clean], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?)));
                match row {
                    Ok(r) => {
                        let doms: Value = r.2.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(json!({}));
                        emit_rob(json!({"pmid": r.0, "tool": r.1, "domains": doms, "overall": r.3, "note": r.4.unwrap_or_default(), "updated": r.5.unwrap_or_default()}));
                    }
                    Err(_) => err_exit_val(json!({"error": format!("no RoB judgement for PMID {clean}")}), 1),
                }
                return;
            }
            let tool_l = tool.clone().unwrap_or_default().trim().to_lowercase();
            let ov_l = overall.clone().unwrap_or_default().trim().to_lowercase();
            if !ROB_TOOLS.contains(&tool_l.as_str()) {
                err_exit(&format!("bad --tool {:?}: use one of {}", tool, ROB_TOOLS.join(", ")), 2);
            }
            if !ROB_OVERALLS.contains(&ov_l.as_str()) {
                err_exit(&format!("bad --overall {:?}: use one of {}", overall, ROB_OVERALLS.join(", ")), 2);
            }
            let mut new_domains: Map<String, Value> = Map::new();
            for d in &domains {
                let Some((k, v)) = d.split_once('=') else { err_exit(&format!("bad --domain {d:?}: use K=V (e.g. -d D1=low)"), 2) };
                let (k, v) = (k.trim(), v.trim());
                if k.is_empty() || v.is_empty() {
                    err_exit(&format!("bad --domain {d:?}: use K=V (e.g. -d D1=low)"), 2);
                }
                new_domains.insert(k.to_string(), Value::String(v.to_string()));
            }
            // Merge with previous.
            let mut merged = new_domains.clone();
            let mut prev_note = String::new();
            if let Ok(row) = conn.query_row("SELECT domains, note FROM rob WHERE pmid=?1", [&clean], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))) {
                if let Some(doms) = row.0.and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|v| v.as_object().cloned()) {
                    for (k, v) in doms {
                        merged.entry(k).or_insert(v);
                    }
                    // Original: merged = prev domains, then update with new.
                    for (k, v) in &new_domains {
                        merged.insert(k.clone(), v.clone());
                    }
                }
                prev_note = row.1.unwrap_or_default();
            }
            let final_note = note.clone().unwrap_or(prev_note);
            let updated = now_iso();
            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO rob (pmid, tool, domains, overall, note, updated) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![clean, tool_l, serde_json::to_string(&Value::Object(merged.clone())).unwrap(), ov_l, final_note, updated],
            ) {
                err_exit(&e.to_string(), 1);
            }
            emit_rob(json!({"pmid": clean, "tool": tool_l, "domains": merged, "overall": ov_l, "note": final_note, "updated": updated}));
        }
        Commands::Trials { query, max_results, status, full } => {
            let clean = match validate_query(&query) {
                Ok(c) => c,
                Err(e) => err_exit(&e, 2),
            };
            if max_results < 1 {
                err_exit(&format!("Invalid --max-results {max_results}: must be >= 1."), 2);
            }
            let valid = ["RECRUITING", "ACTIVE_NOT_RECRUITING", "COMPLETED", "NOT_YET_RECRUITING"];
            if let Some(s) = &status {
                if !valid.contains(&s.to_uppercase().as_str()) {
                    err_exit(&format!("Invalid --status '{s}'. Valid: {}", valid.join(" | ")), 2);
                }
            }
            match trials(&client, &clean, max_results as usize, status.as_deref(), full) {
                Ok((total, out)) => {
                    let n = out.len();
                    emit(&json!({"q": clean, "total": total, "n": n, "r": out}));
                }
                Err(e) => err_exit(&e, 1),
            }
        }
        Commands::Watch { name, query, max_results, list_only, forget } => {
            let conn = match rusqlite::Connection::open(&db) {
                Ok(c) => c,
                Err(e) => err_exit(&e.to_string(), 1),
            };
            if list_only {
                let mut rows_out = Vec::new();
                if let Ok(mut stmt) = conn.prepare("SELECT name, query, last_run FROM saved_queries ORDER BY name") {
                    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))) {
                        for r in rows.flatten() {
                            rows_out.push(json!({"name": r.0, "query": r.1, "last_run": r.2}));
                        }
                    }
                }
                let n = rows_out.len();
                emit(&json!({"n": n, "r": rows_out}));
                return;
            }
            let Some(name_s) = name else { err_exit("watch requires --name except with --list", 2) };
            if forget {
                let _ = conn.execute("DELETE FROM saved_queries WHERE name=?1", [&name_s]);
                let _ = conn.execute("DELETE FROM seen_items WHERE name=?1", [&name_s]);
                emit(&json!({"forgot": name_s}));
                return;
            }
            let explicit_m = max_results.is_some();
            if explicit_m && max_results.unwrap() < 1 {
                err_exit(&format!("Invalid --max-results {}: must be >= 1.", max_results.unwrap()), 2);
            }
            let existing: Option<(String, Option<i64>)> = conn.query_row("SELECT query, max_results FROM saved_queries WHERE name=?1", [&name_s], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))).ok();
            if let Some(q) = &query {
                let clean = match validate_query(q) {
                    Ok(c) => c,
                    Err(e) => err_exit(&e, 2),
                };
                let run_m = if explicit_m { max_results.unwrap() } else { existing.as_ref().and_then(|(_, m)| *m).unwrap_or(20) };
                let _ = conn.execute(
                    "INSERT INTO saved_queries (name, query, created, last_run, max_results) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(name) DO UPDATE SET query=excluded.query, max_results=excluded.max_results",
                    rusqlite::params![name_s, clean, now_iso(), rusqlite::types::Null, run_m],
                );
            }
            let row: Option<(String, Option<i64>)> = conn.query_row("SELECT query, max_results FROM saved_queries WHERE name=?1", [&name_s], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))).ok();
            let Some((stored_query, stored_m)) = row else {
                err_exit(&format!("no saved query '{name_s}' — pass -q to create it"), 2);
            };
            let run_m = if explicit_m { max_results.unwrap() as usize } else { stored_m.unwrap_or(20) as usize };
            // _run_search: date-sorted twin search.
            let (us, eu) = twin_search(&client, &stored_query, (run_m * 3).max(20), None, None, "date", "", "", 0);
            let mut merged = merge_records(us, eu);
            merged.sort_by(|a, b| b.get("date").and_then(|v| v.as_str()).unwrap_or("").cmp(a.get("date").and_then(|v| v.as_str()).unwrap_or("")));
            merged.truncate(run_m);
            let seen: HashSet<String> = conn.prepare("SELECT pmid FROM seen_items WHERE name=?1").ok().and_then(|mut s| s.query_map([&name_s], |r| r.get::<_, String>(0)).ok().map(|rows| rows.flatten().collect())).unwrap_or_default();
            let new_items: Vec<Value> = merged.iter().filter(|r| !seen.contains(r.get("pmid").and_then(|v| v.as_str()).unwrap_or(""))).cloned().collect();
            let baseline = seen.is_empty();
            let now = now_iso();
            for r in &merged {
                if let Some(p) = r.get("pmid").and_then(|v| v.as_str()) {
                    let _ = conn.execute("INSERT OR IGNORE INTO seen_items (name, pmid, first_seen) VALUES (?1,?2,?3)", rusqlite::params![name_s, p, now]);
                }
            }
            let _ = conn.execute("UPDATE saved_queries SET last_run=?1 WHERE name=?2", rusqlite::params![now, name_s]);
            let mut out_v = json!({
                "name": name_s,
                "query": stored_query,
                "checked": merged.len(),
                "n_new": if baseline { 0 } else { new_items.len() },
                "baseline_established": baseline,
                "r": if baseline { merged.clone() } else { new_items.clone() },
            });
            if baseline {
                out_v["note"] = Value::String("baseline stored — new records will appear on the next run".to_string());
            }
            emit(&out_v);
        }
        Commands::Prisma { name, screened, eligible, included, format } => {
            for (label, val) in [("screened", screened), ("eligible", eligible), ("included", included)] {
                if val < 0 {
                    err_exit(&format!("Invalid --{label} {val}: must be >= 0."), 2);
                }
            }
            let conn = match rusqlite::Connection::open(&db) {
                Ok(c) => c,
                Err(e) => err_exit(&e.to_string(), 1),
            };
            let prisma_stats = |conn: &rusqlite::Connection, qname: &str| -> Value {
                let seen: Vec<String> = conn.prepare("SELECT pmid FROM seen_items WHERE name=?1").ok().and_then(|mut s| s.query_map([qname], |r| r.get::<_, String>(0)).ok().map(|rows| rows.flatten().collect())).unwrap_or_default();
                let identified = seen.len() as i64;
                let (mut cached, mut fulltext, mut with_abs, mut no_text) = (0i64, 0i64, 0i64, 0i64);
                for chunk in seen.chunks(500) {
                    let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                    let sql = format!("SELECT pmid, payload FROM lit_cache WHERE pmid IN ({placeholders})");
                    if let Ok(mut stmt) = conn.prepare(&sql) {
                        let params: Vec<&dyn rusqlite::types::ToSql> = chunk.iter().map(|p| p as &dyn rusqlite::types::ToSql).collect();
                        if let Ok(rows) = stmt.query_map(&params[..], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                            for r in rows.flatten() {
                                cached += 1;
                                let Ok(data) = serde_json::from_str::<Value>(&r.1) else { no_text += 1; continue };
                                let has_abs = data.get("abstract").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false);
                                let body: usize = data.get("sections").and_then(|v| v.as_object()).map(|m| m.values().filter_map(|x| x.as_str()).map(|s| s.len()).sum()).unwrap_or(0);
                                let has_ft = body >= 200;
                                if has_ft { fulltext += 1; }
                                if has_abs { with_abs += 1; }
                                if !has_abs && !has_ft { no_text += 1; }
                            }
                        }
                    }
                }
                json!({"identified": identified, "cached": cached, "fulltext": fulltext, "with_abstract": with_abs, "no_text": no_text, "uncached": identified - cached})
            };
            let (targets, scope): (Vec<(String, Option<String>, Option<String>)>, &str) = if let Some(n) = &name {
                match conn.query_row("SELECT query, last_run FROM saved_queries WHERE name=?1", [n], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))) {
                    Ok((q, l)) => (vec![(n.clone(), q, l)], n.as_str()),
                    Err(_) => err_exit(&format!("no saved query '{n}' — prisma reads watch state; create it with watch -n NAME -q QUERY first"), 2),
                }
            } else {
                let mut t = Vec::new();
                if let Ok(mut stmt) = conn.prepare("SELECT name, query, last_run FROM saved_queries ORDER BY name") {
                    if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))) {
                        for r in rows.flatten() {
                            t.push(r);
                        }
                    }
                }
                (t, "all")
            };
            let mut queries: Vec<Value> = Vec::new();
            let mut totals = json!({"identified": 0, "cached": 0, "fulltext": 0, "with_abstract": 0, "no_text": 0, "uncached": 0});
            for (qname, qtext, qlast) in &targets {
                let st = prisma_stats(&conn, qname);
                for k in ["identified", "cached", "fulltext", "with_abstract", "no_text", "uncached"] {
                    totals[k] = Value::Number((totals[k].as_i64().unwrap_or(0) + st[k].as_i64().unwrap_or(0)).into());
                }
                let mut entry = json!({"name": qname, "query": qtext, "last_run": qlast});
                if let Some(obj) = entry.as_object_mut() {
                    if let Some(st_obj) = st.as_object() {
                        for (k, v) in st_obj {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                if name.is_some() {
                    entry["screened"] = Value::Number(screened.into());
                    entry["eligible"] = Value::Number(eligible.into());
                    entry["included"] = Value::Number(included.into());
                } else {
                    entry["screened"] = json!(0);
                    entry["eligible"] = json!(0);
                    entry["included"] = json!(0);
                }
                queries.push(entry);
            }
            let mut totals_with_user = totals.clone();
            totals_with_user["screened"] = Value::Number(screened.into());
            totals_with_user["eligible"] = Value::Number(eligible.into());
            totals_with_user["included"] = Value::Number(included.into());
            let mut note = "Local state only (seen_items + lit_cache): no network, no invented records. screened/eligible/included are user-supplied counts — export --screen CSVs live outside the DB and are never parsed.".to_string();
            if queries.is_empty() && scope == "all" {
                note = format!("No saved watch queries in this DB (honest zeros). {note}");
            }
            if format == "text" {
                println!("PRISMA 2020 flow — local state only (scope: {scope})");
                if queries.is_empty() {
                    println!("No saved watch queries — all counts 0.");
                }
                for e in &queries {
                    println!("Query '{}': {}{}", e["name"].as_str().unwrap_or(""), e["query"].as_str().unwrap_or(""), if e["last_run"].as_str().is_some() { format!(" (last run {})", e["last_run"].as_str().unwrap_or("")) } else { " (never run)".to_string() });
                    println!("  Identification: records identified (watch baseline) = {}", e["identified"]);
                    println!("  Screening (cache): cached = {} (full text ≥200 chars = {}, with abstract = {}, no text = {}, uncached = {})", e["cached"], e["fulltext"], e["with_abstract"], e["no_text"], e["uncached"]);
                    if name.is_some() {
                        println!("  Eligibility (your counts): screened = {}, eligible = {}", e["screened"], e["eligible"]);
                        println!("  Included (your count): included = {}", e["included"]);
                    }
                }
                if scope == "all" && !queries.is_empty() {
                    println!("Totals: identified = {}, cached = {} (full text = {}, with abstract = {}, no text = {}, uncached = {})", totals["identified"], totals["cached"], totals["fulltext"], totals["with_abstract"], totals["no_text"], totals["uncached"]);
                    println!("Eligibility (your counts): screened = {screened}, eligible = {eligible}");
                    println!("Included (your count): included = {included}");
                }
                println!("Note: {note}");
            } else {
                emit(&json!({"scope": scope, "n": queries.len(), "queries": queries, "totals": totals_with_user, "note": note}));
            }
        }
    }
}
