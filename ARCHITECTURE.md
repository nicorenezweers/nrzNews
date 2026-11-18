# Architecture

nrzNews is one Rust program. It serves the JSON API, the web interface (a Dioxus/WebAssembly app embedded at build time), the font and icons, and does all background work: fetching feeds and cleaning up old articles. All data lives in a single SQLite database.

```
┌─────────────── Browser ───────────────┐
│  Dioxus app (WASM, rust-ui components) │
└──────────────────┬────────────────────┘
                   │ /api/v1 (session cookie)
┌──────────────────▼─────────────────────────────────────────────────────┐
│ Rocket 0.5                                                             │
│  routes/api/*   auth, me, admin, folders, feeds, articles, opml, ai    │
│  routes/spa + routes/assets (embedded UI, locales, font, icons)        │
│  auth/       sessions, guards (user, admin, same origin), throttle     │
│  services/   users, settings, folders, feeds, articles, assistant      │
│  feeds/      fetch (conditional GET) · parse (RSS, Atom, JSON Feed)    │
│              discover · sanitize (ammonia) · opml · refresher          │
│  ai.rs       any provider: OpenAI chat protocol + Anthropic Messages   │
└──────────────────┬─────────────────────────────────┬───────────────────┘
                   │ sqlx                            │ reqwest (rustls)
┌──────────────────▼────────────────┐   ┌────────────▼───────────────────┐
│ nrznews.db (SQLite, WAL, FTS5)     │   │ feeds, websites, AI providers  │
│ users, sessions, folders, feeds,  │   └────────────────────────────────┘
│ articles + articles_fts, settings │
└───────────────────────────────────┘
```

## Decisions

| Area | Choice | Why |
|---|---|---|
| Server | Rocket 0.5.1 | Typed request guards for sessions, admins and the origin check; fairings for security headers and the background refresher; graceful shutdown on `SIGTERM` |
| Storage | One SQLite file through **sqlx 0.9** (bundled SQLite, WAL, migrations compiled in from `src/db/migrations.rs`) | Backup = one file; no database server to run |
| Ownership | Feeds belong to one user (like Miniflux); read and starred state live directly on the article | Simple queries and strict separation between accounts, no join table; the same feed subscribed by two people is fetched twice, which is cheap with conditional GET |
| Fetching | `reqwest` with rustls and gzip/brotli/deflate, one shared client, bounded concurrency (`NRZNEWS_FETCH_CONCURRENCY`), 32 MiB limit, only `http`/`https` | Connection reuse; a broken or hostile server can't fill the memory |
| Refresh | `If-None-Match` / `If-Modified-Since` (`etag`, `last_modified`), `next_fetch_at` per feed, `error_count` for backoff | Unchanged feeds cost one small request; failing feeds are asked less and less often instead of every round |
| Parsing | `feed-rs` for RSS (0.9x, 1.0, 2.0), Atom and JSON Feed; stable GUID from the entry ID, else the link, else a hash of title and date | One parser for all formats; no duplicates when a feed rewrites itself |
| Discovery | The URL itself, then `<link rel="alternate">` (`scraper`), then common paths (`/feed`, `/rss.xml`, `/index.xml`, …) probed in parallel | Users paste a website, not a feed URL |
| Content | `ammonia` sanitizes the article HTML (links get `target="_blank" rel="noopener noreferrer"`, relative URLs resolved); a plain-text copy feeds search, excerpts, reading time and AI | Article HTML from foreign sites is never trusted |
| Search | SQLite **FTS5** external-content table over title and text, `unicode61 remove_diacritics 2`, kept in sync by triggers | Umlauts and accents folded (`Muller` finds *Müller*); no extra service |
| Interface | **Dioxus 0.7** (Rust → WebAssembly) with components adapted from **rust-ui**, Tailwind via `dx`; built with `cargo xtask ui` (`dx bundle`, stylesheet source in `ui/tailwind.rs`) and embedded by `build.rs` (gzip) | One binary, no CDN, works offline; the UI talks only to `/api/v1` |
| Shared types | `types/` (`nrznews-types`): every request and response struct, used by server and UI | Both sides can't drift apart; the endpoint table lives in its doc comment |
| AI | `ai.rs`: provider catalog, OpenAI chat protocol plus Anthropic Messages; summaries stored on the article, the overview and Dieter answer with numbered sources (`[1]`, `[2]`) that the UI links | Works with cloud and local models; asking twice costs nothing |
| i18n | German source strings as keys, catalogs in `locales/<lang>.json` embedded by `build.rs`; server messages are German and translated on the client | Readable code; one place for all texts |

## Data model (migrations)

- `0001_init`:
  - `users` (username, display name, Argon2id hash, role `admin`/`member`, disabled, preferences as JSON)
  - `sessions` (SHA-256 of the token, user, expiry, user agent)
  - `app_settings` (instance settings as JSON: registration, refresh interval, retention, AI, cookie key)
  - `folders` (per user, ordered)
  - `feeds` (per user, optional folder, title and original title, feed and site URL, icon, `etag`, `last_modified`, `next_fetch_at`, `last_error`, `error_count`)
  - `articles` (per feed and user, GUID, URL, title, author, sanitized HTML, plain text, image, published/fetched time, read and starred with timestamps, AI summary and model)
  - `articles_fts` (FTS5 over title and text) with insert/update/delete triggers

Times are Unix seconds (UTC). Read articles that are not starred are removed after the retention period (60 days by default, 0 = never).

## Security

- Passwords with **Argon2id**; at least 8 characters
- Sessions: a random 32-byte token in a private (encrypted) cookie, `HttpOnly`, `SameSite=Lax`, optional `Secure`; the database only stores its SHA-256. 30 days, extended while in use
- The cookie key is generated on first start and kept in the database (or set with `NRZNEWS_SECRET_KEY`)
- Changing requests whose `Origin` doesn't match the host are rejected (CSRF); login and registration check it too
- Login throttling per user name and IP address (10 failures in 15 minutes → `429`)
- Roles: the first account is admin; there is always at least one active admin, and admins can't demote, disable or delete themselves
- CSP: scripts only from the server (plus `wasm-unsafe-eval` and `unsafe-eval` for the WASM app), no inline scripts, `connect-src 'self'`, `frame-ancestors 'none'`; images and media from article sites are allowed, with `Referrer-Policy: no-referrer` so those sites don't learn who reads what
- Article HTML sanitized with ammonia before it is stored
- AI keys are stored in the database and never returned to the browser (only a hint such as `sk-or-v1-0…cdef`)

## Build

`docker/nrznews.build` (the Dockerfile) stages:

1. `ui`: installs `dioxus-cli` and builds the WebAssembly interface (`dx bundle --web --release`) once on the build machine
2. `build`: Rust release build, cross-compiled for amd64/arm64 (bundled SQLite and ring with the target's C compiler); embeds the interface, `locales/` and `static/`
3. `binaries`: only the program (`nrznews`), exported for the release assets
4. `runtime`: Debian bookworm-slim with `ca-certificates` and `tzdata`, non-root user `1000`, `/data` volume, health check (`nrznews healthcheck`)

`.github/workflows/release.yml` builds `binaries` and `runtime` for `linux/amd64` and `linux/arm64` when a release is published, pushes `ghcr.io/<owner>/nrz-news:<version>` and `:latest`, and attaches the programs and `SHA256SUMS` to the release. `.github/workflows/ci.yml` runs `cargo test` for the server and `cargo check --target wasm32-unknown-unknown` for the UI on every push and pull request.

## Tests

`cargo test` runs the unit tests in the modules (parsing, sanitizing, discovery, OPML, AI protocol, …) and the integration tests in `tests/`. Those start the server with Rocket's local client on an empty, temporary data folder with background work switched off, create accounts, log in and call `/api/v1` exactly like the UI does: schema and migrations, accounts, sessions and permissions, folders, feeds, articles and search.
