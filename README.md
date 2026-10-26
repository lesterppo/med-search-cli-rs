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
| `fetch` | Implemented | Single + batch (≤50), sections, smart truncation, cache + synchronous stale refresh, PMC/EuropePMC full text, Unpaywall fallback; date + study type exact vs Python for all 4 probe PMIDs; fetch text byte-identical incl. per-section (fix run) |
| `cache-stats` | Implemented | Byte-identical counts in deep test |
| `search-cache` | Implemented | FTS5 snippets identical in deep test; LIKE fallback if FTS5 unavailable |
| `mesh` | Implemented | Labels, UIs, trees, scope, counts identical for aspirin probe |
| `related` | Implemented | Neighbor lists identical for 2 older PMIDs |
| `citedby` | Implemented | Hit counts + lists identical for 2 older PMIDs |
| `refs` | Implemented | EuropePMC + OpenAlex fallback; identical for seed PMID |
| `export` | Implemented | bibtex/ris/csv/json; byte-identical lengths for seed PMID; `--screen` CSV byte-identical incl. RoB columns (fix run) |
| `reconcile` | Implemented | n/agree/kappa/disagreements/skipped identical |
| `rob` | Implemented | Upsert/get/list, domain merge; `updated` is microsecond ISO, like Python |
| `trials` | Implemented | Totals + NCT lists identical for aspirin probe |
| `watch` | Implemented | Baseline + second-run new-paper detection + seen-items + PRISMA state; date-sorted twin search |
| `prisma` | Implemented | Local-state counts identical in deep test |

## Known divergences (deliberate or documented, not hidden)

- **Deterministic full-text contract (both sides).** Full-text source
  priority is PubMed first if the body is ≥200 characters, then
  EuropePMC if ≥200, then Unpaywall fallback. This replaces the
  original's parallel first-finisher-wins (nondeterministic) behaviour;
  the fixed Python clone implements the same contract.
- **Stale cache: synchronous refresh (both sides).** On a stale hit,
  both sides attempt a synchronous refresh before returning: success
  returns the fresh record with `refreshed: true` and re-caches it;
  failure returns the stale record with `cached_stale: true`. This
  replaces the original's background-thread refresh, which usually died
  at process exit.
- **EuropePMC `sort=date` was returning HTTP 503** ("Search service is
  temporarily unavailable") during the original testing; both binaries
  degrade the same way (PubMed leg still fills `search -S date` /
  `watch`). Re-checked in the fix run (2026-10-26): no 503 observed,
  both sides return identical date-ordered results; the shared
  retry/degrade path is retained.
- **Clap vs click help layout** differs structurally (usage line lists
  required args explicitly, `-h` alongside `--help`, long-help line
  layout, top-level list not truncated with `...`), but flags, flag
  order, and descriptions match for all 14 commands. Argument errors
  exit 2 on both.
- **Error JSON spacing (hypothetical only):** Rust error JSON escapes
  non-ASCII exactly like Python `json.dumps` and is byte-identical on
  all six tested error cases; a Python error site using default
  (spaced) separators would differ by spacing only.

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
