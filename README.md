# med-search-cli-rs

AI-native biomedical literature search CLI in Rust — twin-track
**PubMed + EuropePMC** search, paper fetch, citation chasing, MeSH lookup,
reference export, trials, and systematic-review workflow state (RoB,
reconcile, watch, PRISMA).

Rust port of [lesterppo/med-search-cli](https://github.com/lesterppo/med-search-cli)
(v3, 2,862-line Python CLI). The original repo is untouched; this is a
separate repo, built from the original's code, not by amending it.

## Why Rust

- **Single binary, no venv.** The Python original needs click + pymed +
  pandas-class dependencies and a venv; this ships as one 8.3 MB binary
  (`med-search-rs`) with `cargo install --path .`.
- **Startup, not network.** Live calls are network-bound on both sides, but
  local work is ~50× faster: `--version` 0.005 s (Rust) vs 0.25–0.36 s just
  to import click + pymed; cached `fetch` 0.011 s vs 0.38–0.75 s (Python).
- **No pymed.** PubMed is called directly via NCBI E-utilities
  (esearch/efetch/esummary/elink) with XML parsed in Rust; EuropePMC,
  MeSH, OpenAlex, and ClinicalTrials.gov are called as plain REST/JSON.
  That removes a whole dependency layer — and it forced two exactness
  wins documented in DEEP_TEST.md (pymed's date rule, word-boundary study
  types).

## Install

```bash
git clone https://github.com/lesterppo/med-search-cli-rs.git
cd med-search-cli-rs
cargo build --release
# binary: target/release/med-search-rs
# or: cargo install --path .
```

Requires a Rust toolchain (1.99 tested). No API key needed; set
`NCBI_API_KEY` for higher NCBI rate limits, `MED_SEARCH_EMAIL` for
Unpaywall/NCBI identification, `MED_SEARCH_DB` for the SQLite cache path
(default `pubmed_agent_cache.db`), `MED_SEARCH_TTL` for cache TTL in days
(default 30).

## Usage

```bash
med-search-rs search -q "aspirin" -m 5
med-search-rs search -q "aspirin" -m 5 -T RCT -S citations -C 10
med-search-rs fetch -p 42924359
med-search-rs fetch -p "42924359,35139578" -s results -l 3000
med-search-rs cache-stats
med-search-rs search-cache -q aspirin
med-search-rs mesh aspirin -l 3
med-search-rs related -p 35139578 -m 5
med-search-rs citedby -p 35139578 -m 5
med-search-rs refs -p 35139578 -m 5
med-search-rs export -p 42924359 -F bibtex
med-search-rs export -p 42924359 -F csv --screen -o screen.csv
med-search-rs reconcile screen.csv
med-search-rs rob -p 12345678 --tool rob2 -d D1=low --overall low
med-search-rs trials -q aspirin -m 5 --full
med-search-rs watch -n aspirin-watch -q aspirin -m 5
med-search-rs prisma -n aspirin-watch --screened 10 --eligible 3 --included 2
```

All search/fetch-style commands emit compact JSON on stdout, exactly like
the original (same keys, same error shapes: validation errors on stderr
with exit 2, upstream failures on stderr with exit 1).

## Parity table (verified in DEEP_TEST.md, 2026-10-26)

| Command | Status | Notes |
|---|---|---|
| `search` | Implemented | Twin-track, dedup, interleave, study-type pushdown, min-citations, sort, verbose pool diagnostics; record keys identical to Python |
| `fetch` | Implemented | Single + batch (≤50), sections, smart truncation, cache + stale flag, PMC/EuropePMC full text, Unpaywall fallback; date + study type now exact vs Python for all 4 probe PMIDs |
| `cache-stats` | Implemented | Byte-identical counts in deep test |
| `search-cache` | Implemented | FTS5 snippets identical in deep test; LIKE fallback if FTS5 unavailable |
| `mesh` | Implemented | Labels, UIs, trees, scope, counts identical for aspirin probe |
| `related` | Implemented | Neighbor lists identical for 2 older PMIDs |
| `citedby` | Implemented | Hit counts + lists identical for 2 older PMIDs |
| `refs` | Implemented | EuropePMC + OpenAlex fallback; identical for seed PMID |
| `export` | Implemented | bibtex/ris/csv/json; byte-identical lengths for seed PMID in run 2; `--screen` CSV picks up RoB |
| `reconcile` | Implemented | n/agree/kappa/disagreements/skipped identical |
| `rob` | Implemented | Upsert/get/list, domain merge; `updated` is second-precision ISO (Python: microseconds) |
| `trials` | Implemented | Totals + NCT lists identical for aspirin probe |
| `watch` | Implemented | Baseline + seen-items + PRISMA state; date-sorted twin search |
| `prisma` | Implemented | Local-state counts identical in deep test |

## Known divergences (deliberate or documented, not hidden)

- **Full-text attempts are sequential, PubMed-first.** The original runs
  PMC and EuropePMC full-text fetches in parallel and takes the first
  finisher (nondeterministic source). Rust tries PubMed, then EuropePMC,
  deterministically. The returned text and `source` matched Python in
  every deep-test case, but a race could theoretically differ.
- **Stale cache is returned without a background refresh.** The original
  submits a background refresh for stale entries (which usually dies at
  process exit anyway). Rust returns the stale record with
  `cached_stale: true` and does not refresh. A fresh fetch after TTL
  expiry re-caches normally.
- **Fetch text length can differ by tens of characters** (seed PMID:
  5,634 vs 5,680 chars) from section-allocation/truncation edge cases;
  title, date, study type, and source matched exactly.
- **EuropePMC `sort=date` was returning HTTP 503** ("Search service is
  temporarily unavailable") during testing; both binaries degrade the
  same way (PubMed leg still fills `search -S date` / `watch`).
- **RoB `updated` timestamps** are second-precision; Python emits
  microseconds. No other field differs.
- **Clap vs click help/usage text** differs in wording (same flags).
  Argument errors exit 2 on both.

## Environment notes (from the earlier Rust ports on this VM)

- rustls is configured with the system intercept CA
  (`/etc/ssl/certs/hatch-egress-ca.pem` when present) in addition to
  webpki roots; without it HTTPS fails with `UnknownIssuer` behind the
  TLS-intercepting proxy.
- Bracketed IPv6 entries in `NO_PROXY` are stripped in-process at
  startup (they crash httpx on the Python side and confuse proxy
  handling generally).
- roxmltree rejects documents with a DTD, and every PubMed efetch
  response carries a DOCTYPE — the port strips the DOCTYPE (declaration
  only, never body text) before parsing. Two regressions from this were
  found and fixed by the deep test; see DEEP_TEST.md §4.

## License

MIT — same as the original (LICENSE copied).
