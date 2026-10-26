# Deep test: med-search-rs vs med_search_cli.py (Python original)

**Date:** 2026-10-26 (Asia/Hong_Kong). **Method:** both binaries run
back-to-back in the same session, separate SQLite DBs per binary
(`rs.db` / `py.db`, fresh), no API keys set (`NCBI_API_KEY` unset, so
both sides use the unauthenticated NCBI rate). Python side: the original
`med_search_cli.py` from a scratch copy in `~/workspace/med-search-scratch`
(venv: click + pymed) — the original tree was never modified. Rust side:
`target/release/med-search-rs` built from this repo (8.3 MB).

**Verdict: PASS with documented minor divergences.** All 14 commands run
and produce the same contract as Python. Two deep-test runs were done:
run 1 found 3 real bugs (all fixed, §4); run 2 — the numbers below —
is the clean run on the fixed binary, plus targeted re-checks for the
fixed date/study-type cases (§3).

## 1. Comparison table (run 2, fixed binary, unless noted)

| Case | Rust / Python | Verdict |
|---|---|---|
| `search aspirin -m 3` | PMIDs 42924359, 42922442, 42921697 / identical, same order | ✅ exact (PMIDs + titles + record keys) |
| Record keys (search) | 19 keys both, identical set | ✅ exact |
| `fetch 42924359` cold | title equal, source `pubmed_ft` both, text 5,634 / 5,680 chars | ✅ except text length (see §2) |
| `fetch 42924359` cached | `cached: true` both, same source | ✅ exact |
| `related 42924359` (new paper) | n=0 / n=0 | ✅ exact (empty) |
| `citedby 42924359` (new paper) | n=0 / n=0 | ✅ exact (empty) |
| `refs 42924359` | n=5 / n=5, PMIDs 35139578, 33945565, 38701053, 37805449, 41002006 identical | ✅ exact |
| `related 35139578` | n=5 / n=5, lists identical | ✅ exact |
| `citedby 35139578` | n=5, hit_count 27 / identical | ✅ exact |
| `related 18222333` | n=5 / n=5, lists identical | ✅ exact |
| `citedby 18222333` | n=5, hit_count 12 / identical | ✅ exact |
| `export bibtex 42924359` | 547 / 547 bytes | ✅ exact length, title in both |
| `export ris 42924359` | 462 / 462 bytes | ✅ exact length |
| `export csv 42924359` | 415 / 415 bytes | ✅ exact length |
| `export json 42924359` | 2,813 / 2,813 bytes, keys identical | ✅ exact |
| `mesh aspirin -l 3` | labels Aspirin / Aspirin, Dipyridamole Drug Combination / Asthma, Aspirin-Induced; UIs D001241, D000068342, D055963 | ✅ exact |
| `trials aspirin -m 3` | total 2,982 / 2,982; NCTs NCT07437456, NCT01102439, NCT04872283 identical | ✅ exact |
| `search aspirin -T RCT -m 3` | n=3 / n=3, types [RCT] both, overlap 3/3 | ✅ exact |
| `rob` upsert / get / list (PMID 12345678, rob2, D1=low, low) | identical except `updated` precision (seconds vs microseconds) | ✅ wording/precision only |
| `reconcile` screen.csv (2 judged, 1 disagree, 1 skipped) | n=2, agree=1, disagree=1, agree_pct=50.0, kappa=0.0, disagreements=[456], skipped=1, both | ✅ exact |
| `watch -n aspirin-watch -q aspirin -m 3` (baseline) | checked=3, n_new=0, baseline=true, both | ✅ exact |
| `prisma` (after watch baseline) | identified=3, cached=0, uncached=3, both | ✅ exact |
| `cache-stats` (after 1 fetch each) | total=1, stale=0, fts=1, both | ✅ exact |
| `search-cache aspirin` | Same PMID + title/abstract snippets (FTS5 `<b>` highlighting) both | ✅ exact |
| Bad PMID `fetch abc` / `related xyz` | exit 2 both, same error text (Python escapes — as `\u2014`, same parsed string) | ✅ exact as parsed JSON |
| Empty query / operator-only query | exit 2 both, identical error JSON | ✅ exact |
| `search -m 0` | exit 2 both, identical error | ✅ exact |
| `fetch 999999999` (nonexistent) | exit 1 both, identical error | ✅ exact |

## 2. Minor divergences (not passes disguised as passes)

- **Fetch text length, seed PMID:** 5,634 (Rust) vs 5,680 (Python) chars.
  Title, date, study type, and source matched exactly after the §4
  fixes; the 46-char gap is in section-allocation/truncation edge
  cases and was not chased to the last character.
- **RoB `updated`:** Rust writes second-precision ISO
  (`2026-10-26T03:55:24+00:00`); Python writes microseconds
  (`...24.402753+00:00`). Same field, same meaning.
- **Error JSON escaping:** Python's `json.dumps` escapes non-ASCII
  (e.g. em-dash as `\u2014`); Rust emits UTF-8. Parsed JSON is identical.
- **Live counts can drift** between back-to-back calls (new papers
  indexed mid-run); the aspirin search in run 1 returned different
  PMIDs on each side for that reason, not because of a port bug — run 2
  (minutes apart, same index state) matched exactly. Trials total
  (2,982) matched in both runs.

## 3. Date + study-type re-check (after §4 fixes)

These were wrong in run 1 and are exact now, verified individually:

| PMID | Rust date / study | Python date / study |
|---|---|---|
| 42924359 | 2026-10-25 / Observational Study | 2026-10-25 / Observational Study |
| 42892330 | 2026-10-18 / Meta-Analysis | 2026-10-18 / Meta-Analysis |
| 18222333 | 2008-01-29 / Case Reports | 2008-01-29 / Case Reports |
| 35139578 | 2022-02-10 / Review | 2022-02-10 / Review |

The date rule is pymed's own: `PubMedPubDate[@PubStatus='pubmed']`
(Year, Month/Day defaulting to 1), read from pymed's source in the
scratch venv — not JournalIssue PubDate, not DateRevised.

## 4. Bugs found by the deep test and fixed before publishing

1. **roxmltree rejects DTD documents.** Every PubMed efetch response
   carries a DOCTYPE, so the Rust PubMed leg returned 0 records in run 1
   (search was EuropePMC-only, `fetch` failed on EuropePMC-only PMIDs,
   `watch` returned 0). Fix: strip the DOCTYPE declaration before
   parsing.
2. **DOCTYPE stripper searched the whole document for `[`.** Article
   text containing `[` made the stripper give up, leaving the DTD in
   place. Fix: only inspect the DOCTYPE declaration itself for an
   internal subset. Regression test added
   (`strip_doctype_ignores_brackets_in_body`).
3. **Date extraction read PubDate directly under JournalIssue.**
   PubDate is a child of JournalIssue, not a sibling — the first fix
   attempt still returned null dates in the unit test. Fix: descend
   into PubDate, then apply pymed's pubmed-history rule (§3).
4. **Study-type patterns used plain substrings, the original uses
   `\b` word boundaries.** "randomized clinical trials" (plural)
   wrongly matched RCT, and "guidelines" wrongly matched "guideline".
   Fix: word-boundary matching in `detect_study_type`, with the
   original pattern order (observational incl. cross-sectional before
   guideline). Regression tests added for both cases.
5. **EuropePMC `sort=date` was HTTP 503** during testing ("Search
   service is temporarily unavailable") — an upstream outage, not a
   port bug. Both binaries degrade identically: `search -S date` and
   `watch` still return PubMed-leg results (verified after the PubMed
   fix: `search -S date` returns PubMed PMIDs).

## 5. Performance (timed, 5 runs each, min/median)

| Operation | Rust | Python |
|---|---|---|
| `--version` / import click+pymed | 0.005 / 0.006 s | 0.251 / 0.365 s (import only) |
| Cached `fetch` | 0.011 / 0.013 s | 0.381 / 0.751 s |
| Live `search -m 3` | ~3.0 s | ~2.3–2.8 s (network-bound, no claim) |
| Release binary | 8.3 MB, single file | venv (click + pymed + deps) |

Uncached live calls are network-bound on both sides and no speed win
is claimed for them.

## 6. Not verified live

- HTTP 429 handling (backoff code paths are unit-adjacent, not
  deliberately triggered — would require hammering NCBI/EPMC).
- `fetch --section` for each of intro/methods/results/discussion
  individually on a full-text paper (the `all` path and abstract
  fallback were verified; per-section selection shares the same code).
- `export --screen` CSV content beyond the RoB columns (the screen
  flag path is exercised in code review, not in the deep-test run).
- `watch` second run (n_new > 0) — would require waiting for newly
  indexed papers; baseline behaviour and state tables were verified.
