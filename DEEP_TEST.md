# Deep test: med-search-rs vs med_search_cli.py (Python)

**Method:** both binaries run back-to-back in the same session, separate
SQLite DBs per binary (fresh), no API keys set (`NCBI_API_KEY` unset, so
both sides use the unauthenticated NCBI rate). Python side for the fix
run: the fixed Python clone in `~/workspace/med-search-py-fix` (see §7);
the original tree was never modified. Rust side:
`target/release/med-search-rs` built from this repo (8.3 MB).

**Verdict: PASS.** All 14 commands run and produce the same contract as
Python. Every divergence from the original deep test is now fixed and
re-verified live (§2, §7), except the structural clap-vs-click help
differences that cannot be matched without a hand-written help renderer
(§6). The §6 "not verified live" gaps from the original run are now all
closed with real tests (§7).

## 1. Comparison table (fix run, 2026-10-26 ~04:29–04:30 HKT)

| Case | Rust / Python | Verdict |
|---|---|---|
| `search aspirin -m 3` | PMIDs 42924359, 42922442, 42921697 / identical, same order | ✅ exact |
| Record keys (search) | 19 keys both, identical set | ✅ exact |
| `fetch 42924359` cold | title equal, date 2026-10-25, Observational Study, source `pubmed_ft` both, text 5,680 / 5,680 chars | ✅ exact (was 5,634 / 5,680 — fixed, §2) |
| `fetch 42924359` cached | `cached: true` both, same source, text 5,680 / 5,680 | ✅ exact |
| `related 42924359` (new paper) | n=0 / n=0 | ✅ exact (empty) |
| `citedby 42924359` (new paper) | n=0 / n=0 | ✅ exact (empty) |
| `refs 42924359` | n=5 / n=5, PMIDs 35139578, 33945565, 38701053, 37805449, 41002006 identical | ✅ exact |
| `export bibtex 42924359` | 547 / 547 bytes | ✅ exact length, title in both |
| `export ris 42924359` | 462 / 462 bytes | ✅ exact length |
| `export csv 42924359` | 415 / 415 bytes | ✅ exact length |
| `export json 42924359` | 2,813 / 2,813 bytes, keys identical | ✅ exact |
| `mesh aspirin -l 3` | labels Aspirin / Aspirin, Dipyridamole Drug Combination / Asthma, Aspirin-Induced; UIs D001241, D000068342, D055963 | ✅ exact |
| `trials aspirin -m 3` | total 2,982 / 2,982; NCTs NCT07437456, NCT01102439, NCT04872283 identical | ✅ exact |
| `search aspirin -T RCT -m 3` | n=3 / n=3, types [RCT] both, overlap 3/3 | ✅ exact |
| `rob` upsert / get / list (PMID 12345678, rob2, D1=low, low) | identical except wall-clock `updated` values (both microsecond format now) | ✅ exact modulo wall clock |
| `reconcile` screen.csv (2 judged, 1 disagree, 1 skipped) | n=2, agree=1, disagree=1, agree_pct=50.0, kappa=0.0, disagreements=[456], skipped=1, both | ✅ exact |
| `watch -n aspirin-watch -q aspirin -m 3` (baseline) | checked=3, n_new=0, baseline=true, both | ✅ exact |
| `prisma` (after watch baseline) | identified=3, cached=0, uncached=3, both | ✅ exact |
| `cache-stats` (after 1 fetch each) | total=1, stale=0, fts=1, both | ✅ exact |
| `search-cache aspirin` | Same PMID + title/abstract snippets (FTS5 `<b>` highlighting) both | ✅ exact |
| Bad PMID `fetch abc` / `related xyz` | exit 2 both, stderr byte-identical (incl. `\u2014` escaping) | ✅ exact bytes |
| Empty query / operator-only query | exit 2 both, stderr byte-identical | ✅ exact bytes |
| `search -m 0` | exit 2 both, stderr byte-identical | ✅ exact bytes |
| `fetch 999999999` (nonexistent) | exit 1 both, identical error | ✅ exact |

The original run-2 table (pre-fix) is preserved in git history; the only
numbers that changed in the fix run are the ones marked fixed in §2.

## 2. Former divergences — all fixed, with numbers

- **Fetch text length, seed PMID — FIXED.** Root cause, chased to the
  character: Rust `smart_truncate` counted bytes (`str::len`) where
  Python `len()` counts characters. Simulating on the cached results
  section gave Rust 1,746 chars vs Python 1,792 chars — exactly the
  46-char gap, caused by non-ASCII text (e.g. `≥`). Fix: Rust counts
  characters everywhere in truncation (and in `body_len` / `raw`
  length checks), with `i64` arithmetic so `remaining` cannot underflow.
  Verified: `fetch 42924359` now returns exactly 5,680 chars on both
  sides, text byte-identical, cold and cached.
- **RoB `updated` precision — FIXED.** Rust `now_iso()` now emits
  microseconds, matching Python `datetime.now(timezone.utc).isoformat()`.
  Verified: Rust `2026-10-26T04:29:59.358964+00:00` vs Python
  `2026-10-26T04:29:59.903833+00:00` — same format, differing only by
  wall-clock time. `parse_iso()` was also fixed to strip fractional
  seconds (it previously returned `None` for microsecond timestamps,
  so Rust could not read Python-written cache rows).
- **Error JSON escaping — FIXED.** Rust now serializes error JSON with
  Python `json.dumps` semantics: non-ASCII escaped as `\uXXXX` (with
  surrogate pairs above the BMP), compact separators, Python short
  escapes for control characters. Applied to `err_exit`,
  `err_exit_val`, and the verbose pool diagnostic on stderr. Stdout
  success output stays UTF-8, matching Python (`ensure_ascii=False`).
  Verified: all six error cases byte-identical on stderr (bad PMID
  fetch/related, empty query, operator-only query, bad max-results,
  nonexistent PMID). Caveat: Python error sites that use default
  (spaced) separators, if any, would still differ from Rust by spacing
  only; parsed JSON is identical.
- **Full-text fetch order — FIXED (deterministic contract, both
  sides).** The original ran PMC and EuropePMC fetches in parallel and
  took the first finisher (nondeterministic source). Both sides now
  implement the same deterministic contract: PubMed first if the body
  is ≥200 characters, then EuropePMC if ≥200, then Unpaywall fallback.
  Implemented in Rust (`fresh_record()` helper) and in the fixed Python
  clone (`_fetch_fresh_record()`, which also fixes a Python-side defect:
  the old refresh path built a smaller record missing date/commentary/
  pub_types; the new path returns the full record shape).
- **Stale cache — FIXED (synchronous refresh, both sides).** On a stale
  hit, both sides now attempt a synchronous refresh before returning.
  On success the fresh record is returned with `refreshed: true`
  (in-memory only, not saved into the cache payload) and re-cached; on
  failure the stale record is returned with `cached_stale: true`. The
  old Python background-thread refresh (which usually died at process
  exit) was removed. Verified live in §7 (success and failure paths).
- **Help text — ALIGNED as far as clap allows.** Rust now uses
  `help_template = "Usage: {usage}\n\n{about}\n\n{all-args}"` on the
  top command and every subcommand (Usage → description → options,
  like click), `long_about` set to each Python command's full docstring,
  Python's exact argument help text, click-style `value_name`
  (`TEXT`/`INTEGER`), and hidden defaults where click shows none.
  Verified in §7: all 14 commands have identical flag sets in identical
  order, and every Python description (whitespace-normalised) appears
  in the Rust help. Remaining structural differences are listed in §6.

## 3. Date + study-type re-check (from the original run, still exact)

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
5. **EuropePMC `sort=date` was HTTP 503** during the original testing
   ("Search service is temporarily unavailable") — an upstream outage,
   not a port bug. Both binaries degrade identically: `search -S date`
   and `watch` still return PubMed-leg results (verified after the
   PubMed fix: `search -S date` returns PubMed PMIDs). Re-checked in
   the fix run (§7): no 503 observed, both sides return identical
   date-ordered results; the shared retry/degrade path is retained
   (Rust retries 429/5xx up to 4 attempts; Python retries 429/5xx with
   exponential backoff plus jitter).

## 5. Performance (timed, 5 runs each, min/median; original run)

| Operation | Rust | Python |
|---|---|---|
| `--version` / import click+pymed | 0.005 / 0.006 s | 0.251 / 0.365 s (import only) |
| Cached `fetch` | 0.011 / 0.013 s | 0.381 / 0.751 s |
| Live `search -m 3` | ~3.0 s | ~2.3–2.8 s (network-bound, no claim) |
| Release binary | 8.3 MB, single file | venv (click + pymed + deps) |

Uncached live calls are network-bound on both sides and no speed win
is claimed for them.

## 6. Remaining differences (precise reasons, not passes)

- **Clap structural help differences (cannot match without a
  hand-written help renderer):** clap's usage line lists required
  arguments explicitly (e.g. `search [OPTIONS] --query <TEXT>`) where
  click shows only `[OPTIONS]`; clap adds `-h` alongside `--help`
  (click shows `--help` only); clap's long help puts each option's
  description on following lines with blank lines, where click is
  compact single-line; the top-level command list is not truncated
  with `...` as click does. Flags, flag order, and descriptions all
  match (§7); only this layout differs.
- **Error JSON spacing (hypothetical only):** if a Python error site
  uses default (spaced) separators, Rust's compact separators differ
  by spacing only; parsed JSON is identical. No such case was observed
  in the six tested error cases.
- **Live counts can drift** between back-to-back calls (new papers
  indexed mid-run); the aspirin search in run 1 returned different
  PMIDs on each side for that reason, not because of a port bug.
  Trials total (2,982) matched in both runs.
- **Python 429 path is code-inspection only:** Rust's 429 retry is
  unit-tested against a local stub server (§7); Python's `http_request`
  already retries 429 (with `Retry-After` handling) and 5xx with
  backoff, but triggering it live would require hammering NCBI/EPMC,
  so it is stated as inspected, not live-verified.

## 7. Fix run: new cases closing the §6 gaps (2026-10-26, HKT)

Run as `fix_cases.py` against the fixed Rust binary and the fixed
Python clone, fresh DBs. Note on method: the first invocation of this
harness terminated without output midway through §2 (cause
undetermined, no process left); it was re-run unbuffered to completion
and every §2 number reproduced identically in both runs, so the
results below are from the completed run.

- **§1 EuropePMC `sort=date` re-check:** `search -q aspirin -m 3
  -S date` returns PMIDs 42927649, 42927366, 42925734 on both sides,
  identical order (3/3 overlap). No 503 in this run; the shared
  retry/degrade path from §4 item 5 is retained.
- **§2 Per-section fetch (full-text PMID 42924359, all text_equal):**
  all 5,680 chars; intro 4,351; methods 4,714; results 5,995
  (`truncated: true` both); discussion 5,993 (`truncated: true` both);
  source `pubmed_ft` both. Fallback paper 42892330 (no full text):
  all five section requests return the 1,741-char abstract with source
  `fallback` on both sides — per-section selection on the fallback
  path is also identical.
- **§3 `export --screen` full CSV:** byte-identical on both sides.
  Header: `pmid,title,journal,date,study_type,doi,warning,authors,
  include_a,reason_a,include_b,reason_b,consensus,notes,rob_tool,
  rob_overall`; the 42924359 row carries `rob2,low` (from the RoB
  entry) with blank dual-reviewer columns.
- **§4 Watch second run with n_new > 0 (simulation, stated method):**
  baseline `watch -n fix-watch -q aspirin -m 3` gives checked=3,
  n_new=0 on both sides; then one `seen_items` row is deleted from
  each DB with sqlite3 (simulating the watch's view of a newly indexed
  paper — the live index itself cannot be made to change on demand),
  and the second run (no `-q`, state table only) returns n_new=1,
  `r=['42927649']` on both sides, with the deleted PMID back in the
  results. This verifies the second-run detection path, not a live
  index change.
- **§5a Stale refresh, success path:** a cached record is aged past
  TTL; the stale fetch returns `refreshed: true`,
  `cached_stale: null` on both sides, with equal titles and equal
  texts, and the record is re-cached (`cached: true` on the next read,
  both sides).
- **§5b Stale refresh, failure path:** a stale record whose metadata
  cannot be re-fetched (unresolvable PMID state) returns
  `cached_stale: true` with the identical stale title on both sides.
- **§6 Error bytes + RoB precision:** `fetch abc` and empty-query
  stderr byte-identical; RoB `updated` on both sides carries
  microsecond precision (Rust `2026-10-26T04:36:37.894315+00:00`,
  Python `2026-10-26T04:36:38.674580+00:00`).
- **§7 Help flags + descriptions, all 14 commands:** flags_equal=True,
  order_equal=True, no missing/extra flags, and every Python
  description appears in the Rust help — for `export` this holds after
  accounting for one click line-wrap artefact (click wraps
  "independent-screen" across lines as "independent- screen"; the
  Rust help carries the unwrapped word).
- **HTTP 429 (mock, not upstream):** Rust unit test
  `http_get_retries_429_then_succeeds_and_gives_up` runs a local
  `TcpListener` stub server: 429, 429, then 200 must return the body;
  always-429 (4 attempts) must return `None`. NCBI/EPMC were not
  hammered to trigger a real 429. Python's retry path is covered by
  code inspection (§6).

## 8. Test suites (fix run)

- Rust: `cargo test --release` **20/20 passed** (16 original + 4 new:
  `smart_truncate_counts_characters_not_bytes`,
  `json_ascii_escapes_non_ascii_like_python`,
  `now_iso_microseconds_and_parse_iso_fractional`,
  `http_get_retries_429_then_succeeds_and_gives_up`; the 429 test makes
  the suite take ~10 s). `cargo build --release` clean, no warnings.
- Python (fixed clone): `tests/test_pure.py` **10/10 passed** (8
  original + 2 new: `test_fresh_record_prefers_pubmed_and_has_full_shape`,
  `test_stale_fetch_refreshes_synchronously`). Caveat: `pytest` is not
  installed in the scratch venv, so the test functions were run
  directly with a small runner — all 10 passed; that is not a `pytest`
  run.
- Python fixes live in the separate clone
  `~/workspace/med-search-py-fix` (clean git diff, unpushed — see the
  fix report for the diff summary); they are not in the original repo.
