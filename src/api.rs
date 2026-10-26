//! Live API layer: NCBI E-utilities (esearch/efetch/esummary/elink),
//! EuropePMC REST, NLM MeSH lookup, OpenAlex fallback, ClinicalTrials.gov.
//! No pymed, no Python subprocess: all HTTP + XML/JSON parsing in Rust.

use crate::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// XML helpers (roxmltree; tag_name().name() is the local name)
// ---------------------------------------------------------------------------

fn node_text(n: roxmltree::Node) -> String {
    n.descendants().filter(|d| d.is_text()).map(|d| d.text().unwrap_or("")).collect::<String>().trim().to_string()
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Vec<roxmltree::Node<'a, 'a>> {
    n.children().filter(|c| c.is_element() && c.tag_name().name() == name).collect()
}

fn find_all<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Vec<roxmltree::Node<'a, 'a>> {
    n.descendants().filter(|d| d.is_element() && d.tag_name().name() == name).collect()
}

fn month_num(s: &str) -> u32 {
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let t = s.trim();
    if let Ok(n) = t.parse::<u32>() {
        if (1..=12).contains(&n) {
            return n;
        }
    }
    let cap = if t.len() >= 3 { format!("{}{}", &t[..1].to_uppercase(), &t[1..3].to_lowercase()) } else { t.to_string() };
    months.iter().position(|m| *m == cap).map(|i| i as u32 + 1).unwrap_or(1)
}

fn date_from_el(el: roxmltree::Node) -> Option<(String, bool)> {
    let y = child(el, "Year").map(node_text).unwrap_or_default();
    if y.is_empty() {
        return None;
    }
    let m = child(el, "Month").map(node_text).unwrap_or_default();
    let d = child(el, "Day").map(node_text).unwrap_or_default();
    let full = !m.is_empty();
    let mm = if m.is_empty() { 1 } else { month_num(&m) };
    let dd: u32 = d.parse().unwrap_or(1);
    Some((format!("{y}-{mm:02}-{dd:02}"), full))
}

fn pub_date_str(article: roxmltree::Node) -> Option<String> {
    // pymed's exact rule (venv pymed/article.py _extractPublicationDate):
    // PubMedPubDate[@PubStatus='pubmed'], Year required, Month/Day
    // default to 1. That is what the Python side reports, so use it first.
    for h in find_all(article, "PubMedPubDate") {
        if h.attribute("PubStatus") == Some("pubmed") {
            if let Some((s, _)) = date_from_el(h) {
                return Some(s);
            }
        }
    }
    // Fallback when no pubmed history date exists: a full JournalIssue
    // PubDate, then DateRevised, ArticleDate, DateCompleted, and finally
    // a year-only PubDate as YYYY-01-01.
    let pubdate = find_all(article, "JournalIssue").into_iter().next().and_then(|j| child(j, "PubDate")).and_then(date_from_el);
    if let Some((s, true)) = pubdate {
        return Some(s);
    }
    for container in ["DateRevised", "ArticleDate", "DateCompleted"] {
        if let Some(el) = find_all(article, container).into_iter().next() {
            if let Some((s, _)) = date_from_el(el) {
                return Some(s);
            }
        }
    }
    pubdate.map(|(s, _)| s)
}

fn parse_pubmed_article(article: roxmltree::Node) -> Value {
    let mut pmid = String::new();
    for id in find_all(article, "ArticleId") {
        if id.attribute("IdType").unwrap_or("") == "doi" {
            continue;
        }
        // ArticleIdList contains doi + pubmed; take the pubmed one.
        if id.attribute("IdType").unwrap_or("") == "pubmed" {
            pmid = node_text(id);
            break;
        }
    }
    if pmid.is_empty() {
        if let Some(el) = find_all(article, "PMID").into_iter().next() {
            pmid = node_text(el);
        }
    }
    let mut doi: Option<String> = None;
    for id in find_all(article, "ArticleId") {
        if id.attribute("IdType").unwrap_or("") == "doi" {
            doi = Some(node_text(id));
        }
    }
    let title = find_all(article, "ArticleTitle").into_iter().next().map(node_text).unwrap_or_else(|| "Untitled".to_string());
    // Label-preserving abstract.
    let mut abstract_parts: Vec<String> = Vec::new();
    if let Some(abs) = find_all(article, "Abstract").into_iter().next() {
        for t in children(abs, "AbstractText") {
            let text = node_text(t);
            if text.is_empty() {
                continue;
            }
            let label = t.attribute("Label").or_else(|| t.attribute("NlmCategory")).unwrap_or("");
            if !label.is_empty() {
                abstract_parts.push(format!("{}: {}", label.trim().to_uppercase(), text));
            } else {
                abstract_parts.push(text);
            }
        }
    }
    let abstract_text = abstract_parts.join(" ");
    let journal = find_all(article, "Journal")
        .into_iter()
        .next()
        .and_then(|j| child(j, "Title").map(node_text).or_else(|| child(j, "ISOAbbreviation").map(node_text)));
    let mut authors: Vec<String> = Vec::new();
    if let Some(list) = find_all(article, "AuthorList").into_iter().next() {
        for a in children(list, "Author") {
            let last = child(a, "LastName").map(node_text).unwrap_or_default();
            let first = child(a, "ForeName").map(node_text).unwrap_or_else(|| child(a, "Initials").map(node_text).unwrap_or_default());
            let name = format!("{last} {first}").trim().to_string();
            if !name.is_empty() {
                authors.push(name);
            }
            if authors.len() >= 3 {
                break;
            }
        }
    }
    let pub_types: Vec<String> = find_all(article, "PublicationType").into_iter().map(node_text).filter(|t| !t.is_empty()).collect();
    let mesh: Vec<String> = find_all(article, "DescriptorName").into_iter().map(node_text).filter(|t| !t.is_empty()).collect();
    let date = pub_date_str(article);
    let mut rec = json!({
        "pmid": pmid,
        "title": title,
        "date": date,
        "source": "pubmed",
        "abstract": abstract_text,
        "journal": journal,
        "authors": authors,
        "mesh_keywords": mesh,
        "cited_by": Value::Null,
        "in_pmc": false,
        "in_epmc": false,
        "is_open_access": false,
        "study_type": detect_study_type(&title, &abstract_text, &pub_types),
        "pub_types": pub_types,
        "doi": doi,
    });
    let flags = retraction_flags(
        &rec["pub_types"].as_array().unwrap_or(&vec![]).iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect::<Vec<_>>(),
        &[],
    );
    if let Some(obj) = rec.as_object_mut() {
        for (k, v) in flags {
            obj.insert(k, v);
        }
    }
    rec
}

/// roxmltree rejects documents with a DTD ("XML with DTD detected"), and
/// every PubMed/PMC efetch response carries a DOCTYPE. Strip it first;
/// the DTD is never needed for data extraction.
pub fn strip_doctype(xml: &str) -> String {
    let Some(start) = xml.find("<!DOCTYPE") else { return xml.to_string() };
    let rest = &xml[start..];
    // Only the DOCTYPE declaration itself may contain '[' (an internal
    // subset); searching the whole document for '[' is wrong because
    // article text routinely contains brackets. So inspect only up to the
    // first '>' — if that segment has '[', the subset ends at "]>".
    let first_gt = match rest.find('>') {
        Some(p) => p,
        None => return xml.to_string(),
    };
    let end = if rest[..first_gt].contains('[') {
        match rest.find("]>") {
            Some(p) => start + p + 2,
            None => return xml.to_string(),
        }
    } else {
        start + first_gt + 1
    };
    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..start]);
    out.push_str(&xml[end..]);
    out
}

pub fn parse_pubmed_efetch(xml: &str) -> Vec<Value> {
    let cleaned = strip_doctype(xml);
    let Ok(doc) = roxmltree::Document::parse(&cleaned) else { return Vec::new() };
    let root = doc.root_element();
    find_all(root, "PubmedArticle").into_iter().map(parse_pubmed_article).filter(|r| !r["pmid"].as_str().unwrap_or("").is_empty()).collect()
}

pub fn parse_pmc_sections(xml: &str) -> Option<Map<String, Value>> {
    let cleaned = strip_doctype(xml);
    let Ok(doc) = roxmltree::Document::parse(&cleaned) else { return None };
    let root = doc.root_element();
    let body = find_all(root, "body").into_iter().next()?;
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    for sec in find_all(body, "sec") {
        let title_node = child(sec, "title");
        let title_text = title_node.map(node_text).unwrap_or_default().to_lowercase();
        let mut sec_text = node_text(sec);
        if let Some(tn) = title_node {
            let t = node_text(tn);
            if !t.is_empty() {
                if let Some(pos) = sec_text.find(&t) {
                    sec_text = sec_text[pos + t.len()..].trim().to_string();
                }
            }
        }
        if sec_text.is_empty() {
            continue;
        }
        let key = if title_text.contains("intro") || title_text.contains("background") {
            "intro"
        } else if title_text.contains("method") || title_text.contains("patient") || title_text.contains("material") {
            "methods"
        } else if title_text.contains("result") || title_text.contains("find") {
            "results"
        } else if title_text.contains("discuss") || title_text.contains("conclus") || title_text.contains("limit") {
            "discussion"
        } else {
            "other"
        };
        sections.entry(key.to_string()).or_default().push(sec_text);
    }
    let mut out = Map::new();
    for (k, v) in sections {
        let joined = v.join("\n\n").trim().to_string();
        if !joined.is_empty() {
            out.insert(k, Value::String(joined));
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

// ---------------------------------------------------------------------------
// NCBI E-utilities
// ---------------------------------------------------------------------------

fn ncbi_params(extra: &str) -> String {
    let mut p = format!("tool=AgentCLI&email={}", url_encode(&agent_email()));
    if let Ok(key) = std::env::var("NCBI_API_KEY") {
        if !key.is_empty() {
            p.push_str(&format!("&api_key={}", url_encode(&key)));
        }
    }
    if !extra.is_empty() {
        p.push_str(&format!("&{extra}"));
    }
    p
}

pub fn esearch_pubmed(client: &reqwest::blocking::Client, term: &str, retmax: usize, from_date: Option<&str>, to_date: Option<&str>) -> Vec<String> {
    let mut url = format!(
        "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term={}&retmax={}&retmode=json&{}",
        url_encode(term),
        retmax,
        ncbi_params("")
    );
    if let Some(f) = from_date {
        url.push_str(&format!("&mindate={}", f.replace('-', "/")));
    }
    if let Some(t) = to_date {
        url.push_str(&format!("&maxdate={}", t.replace('-', "/")));
    }
    if from_date.is_some() || to_date.is_some() {
        url.push_str("&datetype=pdat");
    }
    let Some(v) = http_get_json(client, &url) else { return Vec::new() };
    v.get("esearchresult").and_then(|r| r.get("idlist")).and_then(|l| l.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default()
}

pub fn efetch_pubmed(client: &reqwest::blocking::Client, pmids: &[String]) -> Vec<Value> {
    if pmids.is_empty() {
        return Vec::new();
    }
    let url = format!(
        "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi?db=pubmed&id={}&retmode=xml&{}",
        url_encode(&pmids.join(",")),
        ncbi_params("")
    );
    let Some(body) = http_get(client, &url) else { return Vec::new() };
    let text = String::from_utf8_lossy(&body);
    parse_pubmed_efetch(&text)
}

pub fn search_pubmed(client: &reqwest::blocking::Client, query: &str, pool: usize, from_date: Option<&str>, to_date: Option<&str>, extra_clause: &str) -> Vec<Value> {
    let optimized = pubmed_query_term(query);
    let term = if extra_clause.is_empty() { optimized } else { format!("({optimized}){extra_clause}") };
    let ids = esearch_pubmed(client, &term, pool, from_date, to_date);
    efetch_pubmed(client, &ids)
}

pub fn pmid_metadata(client: &reqwest::blocking::Client, pmid: &str) -> Option<Value> {
    let recs = efetch_pubmed(client, &[pmid.to_string()]);
    let rec = recs.into_iter().next()?;
    let pub_types: Vec<String> = rec["pub_types"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let commentary = pub_types.iter().any(|p| ["Editorial", "Comment", "Letter", "Published Erratum"].contains(&p.as_str()));
    let mut meta = json!({
        "doi": rec["doi"],
        "title": rec["title"],
        "abstract": if rec["abstract"].as_str().unwrap_or("").is_empty() { Value::String("No abstract available".to_string()) } else { rec["abstract"].clone() },
        "date": rec["date"],
        "study_type": rec["study_type"],
        "commentary": commentary,
        "journal": rec["journal"],
        "authors": rec["authors"],
        "mesh_keywords": rec["mesh_keywords"],
        "pub_types": pub_types,
    });
    let flags = retraction_flags(&pub_types, &[]);
    if let Some(obj) = meta.as_object_mut() {
        for (k, v) in flags {
            obj.insert(k, v);
        }
    }
    Some(meta)
}

pub fn esummary_batch(client: &reqwest::blocking::Client, pmids: &[String]) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    if pmids.is_empty() {
        return out;
    }
    let url = format!(
        "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi?db=pubmed&id={}&retmode=json&{}",
        url_encode(&pmids.join(",")),
        ncbi_params("")
    );
    let Some(v) = http_get_json(client, &url) else { return out };
    if let Some(result) = v.get("result").and_then(|r| r.as_object()) {
        for (k, rec) in result {
            if k == "uids" {
                continue;
            }
            out.insert(k.clone(), rec.clone());
        }
    }
    out
}

pub fn esummary_to_record(pmid: &str, rec: &Value) -> Value {
    let mut doi: Option<String> = None;
    if let Some(aids) = rec.get("articleids").and_then(|v| v.as_array()) {
        for aid in aids {
            if aid.get("idtype").and_then(|v| v.as_str()) == Some("doi") {
                doi = aid.get("value").and_then(|v| v.as_str()).map(|s| s.to_string());
            }
        }
    }
    let pub_types: Vec<String> = rec.get("pubtype").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let title = rec.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
    let mut r = json!({
        "pmid": pmid,
        "title": title,
        "journal": rec.get("fulljournalname").and_then(|v| v.as_str()).or_else(|| rec.get("source").and_then(|v| v.as_str())),
        "date": rec.get("pubdate").and_then(|v| v.as_str()).or_else(|| rec.get("epubdate").and_then(|v| v.as_str())).and_then(iso_from_pubdate),
        "authors": rec.get("authors").and_then(|v| v.as_array()).map(|a| a.iter().take(3).filter_map(|x| x.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()).unwrap_or_default(),
        "study_type": detect_study_type(&title, "", &pub_types),
        "pub_types": pub_types,
        "doi": doi,
    });
    let flags = retraction_flags(&pub_types, &[]);
    if let Some(obj) = r.as_object_mut() {
        for (k, v) in flags {
            obj.insert(k, v);
        }
    }
    r
}

// ---------------------------------------------------------------------------
// EuropePMC
// ---------------------------------------------------------------------------

pub fn search_europepmc(client: &reqwest::blocking::Client, query: &str, pool: usize, from_date: Option<&str>, to_date: Option<&str>, sort: &str, extra_clause: &str, min_citations: i64) -> Vec<Value> {
    let mut q = europepmc_query(query);
    match (from_date, to_date) {
        (Some(f), Some(t)) => q.push_str(&format!(" AND FIRST_PDATE:[{f} TO {t}]")),
        (Some(f), None) => q.push_str(&format!(" AND FIRST_PDATE:[{f} TO 3000-01-01]")),
        (None, Some(t)) => q.push_str(&format!(" AND FIRST_PDATE:[0000-01-01 TO {t}]")),
        (None, None) => {}
    }
    if !extra_clause.is_empty() {
        q.push_str(extra_clause);
    }
    if min_citations > 0 {
        q.push_str(&format!(" AND CITED:[{min_citations} TO 3000000]"));
    }
    let sort_param = match sort {
        "date" => "&sort=FIRST_PDATE%20desc",
        "citations" => "&sort=CITED%20desc",
        _ => "",
    };
    let url = format!(
        "https://www.ebi.ac.uk/europepmc/webservices/rest/search?query={}&format=json&resultType=core&pageSize={}{}",
        url_encode(&q),
        pool,
        sort_param
    );
    let Some(v) = http_get_json(client, &url) else { return Vec::new() };
    let results = v.get("resultList").and_then(|r| r.get("result")).and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for art in results {
        // EuropePMC pmid is usually a string, occasionally a number.
        let pmid = art.get("pmid").map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        }).unwrap_or_default();
        if pmid.is_empty() {
            continue;
        }
        let _ = pmid;
        let title = art.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled").to_string();
        let abstract_text = art.get("abstractText").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let pub_types: Vec<String> = art.get("pubTypeList").and_then(|v| v.get("pubType")).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        let journal = art.get("journalInfo").and_then(|j| j.get("journal")).and_then(|j| j.get("title")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let mut rec = json!({
            "pmid": pmid,
            "title": title,
            "date": art.get("firstPublicationDate"),
            "source": "europepmc",
            "abstract": abstract_text,
            "journal": journal,
            "cited_by": art.get("citedByCount").cloned().unwrap_or(json!(0)),
            "authors": authors_europepmc(&art),
            "mesh_keywords": mesh_europepmc(art.get("meshHeadingList")),
            "pub_types": pub_types,
            "in_epmc": art.get("inEPMC").and_then(|v| v.as_str()) == Some("Y"),
            "in_pmc": art.get("inPMC").and_then(|v| v.as_str()) == Some("Y"),
            "is_open_access": art.get("isOpenAccess").and_then(|v| v.as_str()) == Some("Y"),
            "study_type": detect_study_type(&title, &abstract_text, &pub_types),
            "doi": art.get("doi"),
        });
        let flags = retraction_flags(&pub_types, &epmc_comments(&art));
        if let Some(obj) = rec.as_object_mut() {
            for (k, v) in flags {
                obj.insert(k, v);
            }
        }
        out.push(rec);
    }
    out
}

pub fn fetch_europepmc_fulltext(client: &reqwest::blocking::Client, pmid: &str) -> Option<Map<String, Value>> {
    let url = format!(
        "https://www.ebi.ac.uk/europepmc/webservices/rest/search?query=ext_id:{}%20src:med&format=json&resultType=core",
        pmid
    );
    let v = http_get_json(client, &url)?;
    let results = v.get("resultList").and_then(|r| r.get("result")).and_then(|r| r.as_array())?;
    let paper = results.first()?;
    let pmcid = paper.get("pmcid").and_then(|v| v.as_str())?;
    let ft_url = format!("https://www.ebi.ac.uk/europepmc/webservices/rest/{pmcid}/fullTextXML");
    let body = http_get(client, &ft_url)?;
    let text = String::from_utf8_lossy(&body);
    let secs = parse_pmc_sections(&text)?;
    let body_len: usize = secs.values().filter_map(|v| v.as_str()).map(|s| s.len()).sum();
    if body_len >= 200 { Some(secs) } else { None }
}

pub fn fetch_pubmed_fulltext(client: &reqwest::blocking::Client, pmid: &str) -> Option<Map<String, Value>> {
    let conv = format!(
        "https://www.ncbi.nlm.nih.gov/pmc/utils/idconv/v1.0/?tool=AgentCLI&email={}&ids={}&format=json",
        url_encode(&agent_email()),
        pmid
    );
    let v = http_get_json(client, &conv)?;
    let records = v.get("records").and_then(|r| r.as_array())?;
    let pmcid = records.first()?.get("pmcid").and_then(|x| x.as_str())?;
    let url = format!("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi?db=pmc&id={}&retmode=xml&{}", url_encode(pmcid), ncbi_params(""));
    let body = http_get(client, &url)?;
    let text = String::from_utf8_lossy(&body);
    parse_pmc_sections(&text)
}

pub fn fetch_unpaywall(client: &reqwest::blocking::Client, doi: &str) -> Option<Value> {
    let url = format!("https://api.unpaywall.org/v2/{}?email={}", url_encode(doi), url_encode(&agent_email()));
    let v = http_get_json(client, &url)?;
    if v.get("is_oa").and_then(|x| x.as_bool()) == Some(true) {
        if let Some(pdf) = v.get("best_oa_location").and_then(|b| b.get("url_for_pdf")).and_then(|x| x.as_str()) {
            return Some(json!({"pdf": pdf}));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Merge (search_cmd dedup overlay, minus the interleave — main.rs does that)
// ---------------------------------------------------------------------------

pub fn merge_records(us: Vec<Value>, eu: Vec<Value>) -> Vec<Value> {
    let mut deduped: Vec<Value> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for item in us.into_iter().chain(eu.into_iter()) {
        let pid = item.get("pmid").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if pid.is_empty() {
            continue;
        }
        if let Some(&pos) = index.get(&pid) {
            let existing = &mut deduped[pos];
            for key in ["abstract", "journal", "cited_by", "authors", "mesh_keywords", "pub_types", "in_epmc", "in_pmc", "is_open_access", "doi"] {
                let new_val = item.get(key).cloned();
                if is_empty_val(new_val.as_ref()) {
                    continue;
                }
                if is_empty_val(existing.get(key)) {
                    if key == "cited_by" {
                        existing["cited_by_source"] = item.get("source").cloned().unwrap_or(Value::Null);
                    }
                    existing[key] = new_val.unwrap();
                }
            }
            if existing.get("study_type").and_then(|v| v.as_str()) == Some("Unknown") && item.get("study_type").and_then(|v| v.as_str()) != Some("Unknown") {
                existing["study_type"] = item["study_type"].clone();
            }
            for flag in ["retracted", "expression_of_concern", "corrected"] {
                if item.get(flag).and_then(|v| v.as_bool()) == Some(true) {
                    existing[flag] = Value::Bool(true);
                }
            }
            let item_src = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let exist_src = existing.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !item_src.is_empty() && !exist_src.is_empty() && item_src != exist_src {
                existing["source"] = Value::String("both".to_string());
            }
        } else {
            index.insert(pid, deduped.len());
            deduped.push(record_defaults(item));
        }
    }
    deduped
}

pub fn twin_search(client: &reqwest::blocking::Client, query: &str, pool: usize, from_date: Option<&str>, to_date: Option<&str>, sort: &str, pm_clause: &str, ep_clause: &str, min_citations: i64) -> (Vec<Value>, Vec<Value>) {
    let us = search_pubmed(client, query, pool, from_date, to_date, pm_clause);
    let eu = search_europepmc(client, query, pool, from_date, to_date, sort, ep_clause, min_citations);
    (us, eu)
}

// ---------------------------------------------------------------------------
// MeSH, related, citedby, refs, trials
// ---------------------------------------------------------------------------

pub fn mesh_lookup(client: &reqwest::blocking::Client, term: &str, limit: i64, exact: bool, counts: bool) -> Result<Vec<Value>, String> {
    let limit = limit.min(50);
    let match_kind = if exact { "exact" } else { "contains" };
    let url = format!("https://id.nlm.nih.gov/mesh/lookup/descriptor?label={}&match={}&limit={}", url_encode(term), match_kind, limit);
    let Some(hits) = http_get_json(client, &url) else { return Err("MeSH lookup failed (no response).".to_string()) };
    let hits = hits.as_array().cloned().unwrap_or_default();
    // Term labels (entry terms).
    let mut term_labels: Vec<String> = Vec::new();
    let turl = format!("https://id.nlm.nih.gov/mesh/lookup/term?label={}&match=contains&limit=10", url_encode(term));
    if let Some(tv) = http_get_json(client, &turl) {
        if let Some(arr) = tv.as_array() {
            for t in arr {
                if let Some(l) = t.get("label").and_then(|v| v.as_str()) {
                    term_labels.push(l.to_string());
                }
            }
        }
    }
    let mut out = Vec::new();
    for h in hits {
        let ui = h.get("resource").and_then(|v| v.as_str()).unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
        let mut entry = json!({"ui": ui, "label": h.get("label")});
        if !term_labels.is_empty() {
            entry["terms"] = Value::Array(term_labels.iter().take(6).map(|s| Value::String(s.clone())).collect());
        }
        let durl = format!("https://id.nlm.nih.gov/mesh/{ui}.json");
        if let Some(d) = http_get_json(client, &durl) {
            if let Some(lbl) = d.get("label").and_then(|v| v.get("@value")).and_then(|v| v.as_str()) {
                entry["label"] = Value::String(lbl.to_string());
            }
            let tn = d.get("treeNumber").cloned().unwrap_or(Value::Null);
            let mut trees: Vec<String> = Vec::new();
            match &tn {
                Value::Array(arr) => {
                    for t in arr {
                        let val = t.get("@id").and_then(|v| v.as_str()).unwrap_or_else(|| t.as_str().unwrap_or(""));
                        if !val.is_empty() {
                            trees.push(val.rsplit('/').next().unwrap_or(val).to_string());
                        }
                    }
                }
                Value::String(s) => {
                    if !s.is_empty() {
                        trees.push(s.rsplit('/').next().unwrap_or(s).to_string());
                    }
                }
                Value::Object(_) => {
                    if let Some(id) = tn.get("@id").and_then(|v| v.as_str()) {
                        trees.push(id.rsplit('/').next().unwrap_or(id).to_string());
                    }
                }
                _ => {}
            }
            if !trees.is_empty() {
                entry["tree"] = Value::Array(trees.into_iter().map(Value::String).collect());
            }
            if let Some(sn) = d.get("scopeNote").and_then(|v| v.get("@value")).and_then(|v| v.as_str()) {
                if !sn.trim().is_empty() {
                    let scope: String = sn.chars().take(400).collect();
                    entry["scope"] = Value::String(scope);
                }
            }
        }
        if counts {
            let label = entry.get("label").and_then(|v| v.as_str()).unwrap_or(term).to_string();
            let q = url_encode(&format!("\"{label}\"[MeSH]"));
            let curl = format!("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pubmed&term={q}&rettype=count&retmode=json&{}", ncbi_params(""));
            if let Some(cv) = http_get_json(client, &curl) {
                if let Some(count) = cv.get("esearchresult").and_then(|r| r.get("count")).and_then(|v| v.as_str()).and_then(|s| s.parse::<i64>().ok()) {
                    entry["pubmed_hits"] = Value::Number(count.into());
                }
            }
        }
        out.push(entry);
    }
    Ok(out)
}

pub fn related(client: &reqwest::blocking::Client, pmid: &str, max_results: usize) -> Vec<Value> {
    let url = format!("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/elink.fcgi?dbfrom=pubmed&db=pubmed&id={pmid}&cmd=neighbor&retmode=json&{}", ncbi_params(""));
    let Some(v) = http_get_json(client, &url) else { return Vec::new() };
    let mut neighbors: Vec<String> = Vec::new();
    if let Some(linksets) = v.get("linksets").and_then(|x| x.as_array()) {
        for ls in linksets {
            if let Some(dbs) = ls.get("linksetdbs").and_then(|x| x.as_array()) {
                for ldb in dbs {
                    if ldb.get("linkname").and_then(|x| x.as_str()) == Some("pubmed_pubmed") {
                        if let Some(links) = ldb.get("links").and_then(|x| x.as_array()) {
                            neighbors = links.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).filter(|x| x != pmid).collect();
                        }
                    }
                }
            }
        }
    }
    neighbors.truncate(max_results);
    if neighbors.is_empty() {
        return Vec::new();
    }
    let summ = esummary_batch(client, &neighbors);
    neighbors.iter().filter_map(|p| summ.get(p).map(|r| esummary_to_record(p, r))).collect()
}

pub fn citedby(client: &reqwest::blocking::Client, pmid: &str, max_results: usize) -> Option<(Option<i64>, Vec<Value>)> {
    let url = format!("https://www.ebi.ac.uk/europepmc/webservices/rest/MED/{pmid}/citations?format=json&pageSize={max_results}");
    let v = http_get_json(client, &url)?;
    let hit_count = v.get("hitCount").and_then(|x| x.as_i64());
    let cites = v.get("citationList").and_then(|c| c.get("citation")).and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for c in cites {
        let src = c.get("source").and_then(|x| x.as_str()).unwrap_or("").to_uppercase();
        let cid = c.get("id").and_then(|x| x.as_str()).map(|s| s.to_string());
        out.push(json!({
            "pmid": if src == "MED" { cid.clone().map(Value::String).unwrap_or(Value::Null) } else { Value::Null },
            "id": cid,
            "source": c.get("source"),
            "title": c.get("title"),
            "journal": c.get("journalAbbreviation").and_then(|x| x.as_str()).or_else(|| c.get("journalTitle").and_then(|x| x.as_str())),
            "year": c.get("pubYear"),
            "authors": c.get("authorString").and_then(|x| x.as_str()).unwrap_or("").chars().take(80).collect::<String>(),
            "type": c.get("citationType"),
        }));
    }
    Some((hit_count, out))
}

pub fn refs_openalex(client: &reqwest::blocking::Client, pmid: &str, max_results: usize, degraded: Option<String>) -> (Vec<Value>, Option<String>) {
    let url = format!("https://api.openalex.org/works/pmid:{pmid}?select=id,referenced_works");
    let Some(v) = http_get_json(client, &url) else { return (Vec::new(), degraded) };
    let refs = v.get("referenced_works").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    if refs.is_empty() {
        return (Vec::new(), degraded);
    }
    let ids: Vec<String> = refs.iter().take(max_results).filter_map(|r| r.as_str().map(|s| s.rsplit('/').next().unwrap_or(s).to_string())).collect();
    let q = format!("openalex_id:{}", ids.join("|"));
    let url2 = format!("https://api.openalex.org/works?filter={}&per-page=50&select=id,ids,title,publication_year,doi", url_encode(&q));
    let mut out = Vec::new();
    if let Some(v2) = http_get_json(client, &url2) {
        if let Some(results) = v2.get("results").and_then(|x| x.as_array()) {
            for w in results {
                let mut pmid_val: Option<String> = None;
                if let Some(ids_map) = w.get("ids").and_then(|x| x.as_object()) {
                    if let Some(p) = ids_map.get("pmid").and_then(|x| x.as_str()) {
                        pmid_val = Some(p.rsplit('/').next().unwrap_or(p).to_string());
                    }
                }
                out.push(json!({
                    "pmid": pmid_val,
                    "doi": w.get("doi").and_then(|x| x.as_str()).unwrap_or("").replace("https://doi.org/", ""),
                    "title": w.get("title"),
                    "year": w.get("publication_year").map(|x| x.to_string()).unwrap_or_default(),
                }));
            }
        }
    }
    if !out.is_empty() {
        let note = format!("{}OpenAlex reference list used.", degraded.map(|d| format!("{d} ")).unwrap_or_default());
        return (out, Some(note));
    }
    let ids_only: Vec<Value> = refs.iter().take(max_results).map(|r| json!({"openalex": r})).collect();
    let note = format!("{}OpenAlex IDs only (no metadata resolved).", degraded.map(|d| format!("{d} ")).unwrap_or_default());
    (ids_only, Some(note))
}

pub fn refs(client: &reqwest::blocking::Client, pmid: &str, max_results: usize) -> (Option<i64>, Vec<Value>, Option<String>) {
    let url = format!("https://www.ebi.ac.uk/europepmc/webservices/rest/MED/{pmid}/references?format=json&pageSize={max_results}");
    let mut out: Vec<Value> = Vec::new();
    let mut hit_count: Option<i64> = None;
    let mut degraded: Option<String> = None;
    if let Some(v) = http_get_json(client, &url) {
        if v.get("referenceList").is_some() {
            if let Some(list) = v.get("referenceList").and_then(|r| r.get("reference")).and_then(|x| x.as_array()) {
                for r in list {
                    out.push(json!({
                        "pmid": if r.get("source").and_then(|x| x.as_str()) == Some("MED") { r.get("id").cloned().unwrap_or(Value::Null) } else { Value::Null },
                        "doi": r.get("doi"),
                        "title": r.get("title"),
                        "journal": r.get("journalAbbreviation"),
                        "year": r.get("pubYear"),
                        "authors": r.get("authorString").and_then(|x| x.as_str()).unwrap_or("").chars().take(80).collect::<String>(),
                    }));
                }
            }
            hit_count = v.get("hitCount").and_then(|x| x.as_i64());
        } else {
            degraded = Some("EuropePMC references endpoint returned no reference list.".to_string());
        }
    } else {
        degraded = Some("EuropePMC references endpoint unavailable.".to_string());
    }
    if out.is_empty() {
        let (oa_out, oa_note) = refs_openalex(client, pmid, max_results, degraded);
        return (hit_count, oa_out, oa_note);
    }
    (hit_count, out, degraded)
}

pub fn trials(client: &reqwest::blocking::Client, query: &str, max_results: usize, status: Option<&str>, full: bool) -> Result<(Option<i64>, Vec<Value>), String> {
    let mut params = format!("query.term={}&pageSize={}&countTotal=true", url_encode(query), max_results.min(50));
    if let Some(s) = status {
        params.push_str(&format!("&filter.overallStatus={}", s.to_uppercase()));
    }
    let url = format!("https://clinicaltrials.gov/api/v2/studies?{params}");
    let Some(v) = http_get_json(client, &url) else { return Err("ClinicalTrials.gov request failed.".to_string()) };
    let total = v.get("totalCount").and_then(|x| x.as_i64());
    let mut out = Vec::new();
    if let Some(studies) = v.get("studies").and_then(|x| x.as_array()) {
        for s in studies {
            let prot = s.get("protocolSection").cloned().unwrap_or(json!({}));
            let ident = prot.get("identificationModule").cloned().unwrap_or(json!({}));
            let design = prot.get("designModule").cloned().unwrap_or(json!({}));
            let stat = prot.get("statusModule").cloned().unwrap_or(json!({}));
            let mut entry = json!({
                "nct": ident.get("nctId"),
                "title": ident.get("briefTitle").and_then(|x| x.as_str()).unwrap_or("").chars().take(250).collect::<String>(),
                "status": stat.get("overallStatus"),
                "phase": design.get("phases").and_then(|x| x.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()).unwrap_or("N/A"),
                "enroll": design.get("enrollmentInfo").and_then(|e| e.get("count")),
            });
            if full {
                entry["conditions"] = Value::Array(
                    prot.get("conditionsModule").and_then(|c| c.get("conditions")).and_then(|x| x.as_array()).cloned().unwrap_or_default().into_iter().take(4).collect(),
                );
                entry["interventions"] = Value::Array(
                    prot.get("armsInterventionsModule").and_then(|a| a.get("interventions")).and_then(|x| x.as_array()).cloned().unwrap_or_default().into_iter().take(4).filter_map(|i| i.get("name").and_then(|n| n.as_str()).map(|s| Value::String(s.to_string()))).collect(),
                );
                entry["locations"] = Value::Array(
                    prot.get("contactsLocationsModule").and_then(|c| c.get("locations")).and_then(|x| x.as_array()).cloned().unwrap_or_default().into_iter().take(4).filter_map(|l| l.get("country").and_then(|n| n.as_str()).map(|s| Value::String(s.to_string()))).collect(),
                );
                entry["start"] = stat.get("startDateStruct").and_then(|d| d.get("date")).cloned().unwrap_or(Value::Null);
            }
            out.push(entry);
        }
    }
    Ok((total, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pubmed_efetch_with_doctype() {
        let xml = r#"<?xml version="1.0" ?>
<!DOCTYPE PubmedArticleSet PUBLIC "-//NLM//DTD PubMedArticle, 1st January 2025//EN" "https://dtd.nlm.nih.gov/ncbi/pubmed/out/pubmed_250101.dtd">
<PubmedArticleSet><PubmedArticle><MedlineCitation><PMID Version="1">42892330</PMID><Article><Journal><JournalIssue><PubDate><Year>2026</Year><Month>Oct</Month><Day>17</Day></PubDate></JournalIssue><Title>J Thromb Haemost</Title></Journal><ArticleTitle>Test title aspirin</ArticleTitle><Abstract><AbstractText Label="BACKGROUND">Some background.</AbstractText></Abstract></Article></MedlineCitation></PubmedArticle></PubmedArticleSet>"#;
        let recs = parse_pubmed_efetch(xml);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0]["pmid"], "42892330");
        assert!(recs[0]["abstract"].as_str().unwrap().contains("BACKGROUND: Some background."));
        assert_eq!(recs[0]["date"], "2026-10-17");
    }

    #[test]
    fn strip_doctype_no_doctype_is_identity() {
        assert_eq!(strip_doctype("<a/>"), "<a/>");
    }

    #[test]
    fn strip_doctype_ignores_brackets_in_body() {
        // Regression: a '[' in article text must not stop the strip.
        let xml = "<!DOCTYPE PubmedArticleSet PUBLIC \"-//NLM//DTD\" \"https://example.com/dtd\"><root><a>[bracket] text</a></root>";
        let cleaned = strip_doctype(xml);
        assert!(!cleaned.contains("DOCTYPE"), "{cleaned}");
        assert!(cleaned.contains("[bracket]"), "{cleaned}");
    }
}
