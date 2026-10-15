# ratiod — design

*Date: 2026-10-14 · Status: approved*

A Rust rewrite of [Joal](https://github.com/anthonyraymond/joal) (Apache 2.0): a fake BitTorrent client that
announces fake upload to HTTP trackers for `.torrent` files, without transferring anything.

## 1. Goal and constraints

**What was requested**
- A complete rewrite in Rust, much lighter and faster.
- A modern, pleasant frontend, with torrents added straight from the UI (multi-file drag and drop).
- No authentication system: local use.
- Code hosted in a **private** `xavierl/ratiod` repository on GitHub.

**Usage context**
- Runs in **Docker on a NAS**; the UI is used from devices on the LAN.
- Replaces an existing Joal installation: the config format is free to change, but existing `.client` and
  `.torrent` files must remain reusable.
- No CI and no image registry: the image is built on the NAS (`docker compose build`).

**Success criteria**
1. All 95 Joal `.client` profiles load and produce a valid announce request.
2. For a given profile, the announce request (query + headers) has the same shape as Joal's.
3. An existing `joal-conf` folder (config.json, clients/, torrents/) works when mounted on `/data`.
4. Docker image < 15 MB; idle memory < 30 MB with 200 active torrents.
5. A `.torrent` dropped into the UI appears in the list and is announced within 3 s if a slot is free.

## 2. Scope

**Included (Joal parity + approved additions)**
- Global seeding start/stop, **automatic start** on launch (can be disabled).
- Settings: min/max rate, simultaneous seeds, emulated client, target ratio, keep torrents with 0 leechers.
- List of active torrents with rate, uploaded amount, seeders/leechers, next announce, errors.
- Adding `.torrent` files with a button and **multi-file drag and drop**; deleting a torrent.
- Watching the `torrents/` folder (a file dropped directly on the NAS is picked up).
- Automatic archiving (ratio reached, 0 leechers, 5 consecutive failures).

**Excluded (YAGNI, can be added later without breaking anything)**
- UDP trackers, magnet links, DHT.
- Per-torrent pause/resume or ratio.
- Persistent statistics, history, charts.
- Authentication, HTTPS (LAN use).
- Public IP detection (see §4.4).
- CI, image publishing.

## 3. Architecture

A single Rust binary, `ratiod`, that embeds the compiled frontend. Stack: `tokio`, `axum`, `hyper` + `hyper-rustls`
(HTTP client with no implicit headers, gzip via `flate2`), `serde`/`serde_json`, `notify`, `rust-embed`,
`rand`/`rand_regex`, `sha1`, `tracing`. Frontend: Svelte 5 (runes) + Vite + Tailwind CSS v4, no router.

```
ratiod/
├── Cargo.toml
├── src/
│   ├── main.rs          startup, env, signals, wiring
│   ├── config.rs        AppConfig (config.json) + Settings (env variables)
│   ├── torrent.rs       bencode + MockedTorrent (info_hash, name, size, trackers)
│   ├── bencode.rs       minimal bencode decoder (keeps the raw byte span of the `info` dict)
│   ├── emulation/       .client profiles
│   │   ├── mod.rs       ClientProfile: loading, validation, build_query(), headers()
│   │   ├── algorithm.rs peer_id / key algorithms
│   │   ├── generator.rs refresh policies
│   │   └── urlencode.rs configurable URL encoder
│   ├── tracker.rs       HTTP announce, response parsing, tracker failover
│   ├── bandwidth.rs     random global rate + weighted split + accumulation
│   ├── library.rs       torrents/ + archived/ folders + watcher
│   ├── engine.rs        orchestrator actor (sole owner of the state)
│   └── api.rs           axum routes, SSE, embedded static files
├── clients/             Joal's ~95 .client profiles (embedded)
├── web/                 Svelte app (build → web/dist, embedded)
├── Dockerfile
└── docker-compose.yml
```

### 3.1 Concurrency model

The `engine` is **a single tokio task that owns all the state** (active torrents, queue, bandwidth, client
profile). The other components talk to it through messages:

- `mpsc<Command>` to the engine: `Start`, `Stop`, `UpdateConfig`, `Rescan`, `Delete(infohash)`,
  `AnnounceDone(infohash, Result)`, `Shutdown`. A `Rescan` re-reads the folder and reconciles: new files join the
  queue, files that disappeared are removed (with `stopped`).
- `watch<Snapshot>` from the engine: the complete, serializable state, published on every change and every tick.

HTTP announces run in separate tasks (`tokio::spawn`), capped at 20 concurrent ones by a `Semaphore`. Their result
comes back as `Command::AnnounceDone`. The engine never blocks on the network and uses no locks.

The engine loop runs `tokio::select!` over: commands, a one-second tick (upload accumulation, firing announces
whose time has come) and the global rate refresh (20 min).

### 3.2 Data flow

```
UI ──POST /api/torrents──▶ api ──writes──▶ /data/torrents/x.torrent
                                              │ notify
                                              ▼
                         library ──Rescan──▶ engine ──spawn──▶ tracker (started)
                                                         ▲                    │
                                                         └──AnnounceDone──────┘
engine ──watch<Snapshot>──▶ api ──SSE /api/events──▶ UI
```

An upload from the UI follows the same path as a file dropped by hand: a single source of truth, the folder.

## 4. Components

### 4.1 `config`

**`/data/config.json`**: same keys as Joal, read at startup, rewritten by `PUT /api/config`. If it is missing, it
is created with the values below.

| Key | Type | Default | Validation |
|---|---|---|---|
| `minUploadRate` | u64 (kB/s) | 30 | ≥ 0 |
| `maxUploadRate` | u64 (kB/s) | 170 | ≥ `minUploadRate` |
| `simultaneousSeed` | u32 | 200 | ≥ 1 |
| `client` | string | latest embedded qBittorrent profile | must exist |
| `keepTorrentWithZeroLeechers` | bool | true | — |
| `uploadRatioTarget` | f32 | -1 | `-1` (disabled) or > 0 |

The defaults are Joal's. As in Joal, 1 kB = 1000 bytes.

**Environment variables** (not editable from the UI):

| Variable | Binary default | Docker image default |
|---|---|---|
| `RATIOD_BIND` | `127.0.0.1:5081` | `0.0.0.0:5081` |
| `RATIOD_DATA_DIR` | `./data` | `/data` |
| `RATIOD_PEER_PORT` | random 49152–65534 | `49200` |
| `RATIOD_AUTOSTART` | `true` | `true` |
| `RUST_LOG` | `info` | `info` |

### 4.2 `torrent` and `bencode`

- A home-made bencode decoder (≈150 lines) that keeps the raw byte span of the `info` dict to compute
  `info_hash = SHA-1(raw bytes)`. No re-encoding, so no risk of a wrong hash on a non-canonical torrent.
- `MockedTorrent { info_hash: [u8; 20], name, size, trackers: Vec<Url> }`.
  - `size`: `length`, or the sum of `files[].length`.
  - `trackers`: `announce-list` flattened in order, otherwise `announce`.
  - Only `http`/`https` trackers are kept; a torrent with no HTTP tracker at all is rejected.

### 4.3 `emulation`

Loads profiles **in Joal's `.client` format, unmodified**. Embedded profiles (`clients/`, via `rust-embed`) are
merged with `/data/clients/*.client`: when names are equal, the file from the volume wins.

**Algorithms** (`algorithm.type` field):

| Type | Used for | Behaviour |
|---|---|---|
| `REGEX` | peer_id, key | random string matching `pattern` (`rand_regex`) |
| `RANDOM_POOL_WITH_CHECKSUM` | peer_id | `prefix` padded to 20 characters: each character is `charactersPool[random byte % base]`, the last one makes the sum of indices a multiple of `base` (same computation as `RandomPoolWithChecksumPeerIdAlgorithm.java`) |
| `HASH` | key | `length` random uppercase hex characters |
| `HASH_NO_LEADING_ZERO` | key | same, leading zeros removed |
| `DIGIT_RANGE_TRANSFORMED_TO_HEX_WITHOUT_LEADING_ZEROES` | key | integer in `[inclusiveLowerBound, inclusiveUpperBound]` → hex without leading zeros |

`keyCase` (`upper`/`lower`/`none`) applies to the generated key.

**Refresh policies** (`refreshOn`):

| Value | Behaviour |
|---|---|
| `NEVER` | one value for the whole process lifetime |
| `ALWAYS` | a new value on every announce |
| `TIMED` | a new value every `refreshEvery` seconds |
| `TORRENT_PERSISTENT` | one value per torrent, kept until the torrent is removed |
| `TORRENT_VOLATILE` | one value per torrent, forgotten after the `stopped` announce |
| `TIMED_OR_AFTER_STARTED_ANNOUNCE` | a new value after `refreshEvery` seconds, or after every `started` announce |

**URL encoder**: each byte is kept if it matches `encodingExclusionPattern`, otherwise encoded as `%XX` with the
`encodedHexCase` casing. It applies to the info_hash (20 raw bytes), to the key, and to the peer_id when
`shouldUrlEncode` is true.

**Query building** (same algorithm as `BitTorrentClient.createRequestQuery`):
1. Collapse consecutive `&` in the template.
2. Replace `{infohash} {uploaded} {downloaded} {left} {port} {numwant} {peerid} {key}`.
3. Remove the `…={ip}` / `…={ipv6}` parameters.
4. Event `none`: remove the `…={event}` parameter; otherwise `started` / `stopped`.
5. `numwant` is `numwantOnStop` for `stopped`, `numwant` otherwise.
6. If any `{…}` remains, **the profile is invalid** (rejected at load time, checked with dummy values).
7. Strip leading and trailing `&`.

**Headers**: the profile's ordered list, with `{java}` → `21.0.4`, `{os}` → `Linux`, `{locale}` → `en-US` (fixed,
plausible values). An unknown placeholder makes the profile invalid.

### 4.4 `tracker`

- Request `GET {announce_url}{?|&}{query}`. The query is appended **as is, without re-encoding**: the URL is built
  as a string and then parsed, and a test checks that the path+query sent is byte-for-byte identical.
- `hyper` client (not `reqwest`, which adds an `Accept: */*` header that cannot be removed): only `Host` (first,
  like real clients) then the profile's headers are sent, in Title-Case.
  30 s timeout, gzip accepted if the profile announces it.
- Bencoded response:
  - `failure reason` → error.
  - Otherwise `interval` (required), `complete`, `incomplete` (0 if missing).
  - `seeders = max(0, complete − 1)` (we remove ourselves from the count), `leechers = incomplete`.
- On failure, move on to the next tracker in the list (cycling). The consecutive failure counter is reset on the
  first success.
- **Public IP**: not detected. No profile uses `{ip}`, and Joal already removes `{ipv6}` when the detected IP is an
  IPv4 address (the usual case). This avoids calls to third-party services.
- **Announced port**: `RATIOD_PEER_PORT`. A `TcpListener` really listens on it (connections accepted then closed)
  so the port looks open.

### 4.5 `bandwidth`

An exact port of `BandwidthDispatcher`:
- **Global rate** drawn uniformly in `[min, max)` kB/s (or `max` if min = max), redrawn every 20 min and on every
  config change.
- **Torrent weight**:
  ```
  leechers_ratio = leechers / (seeders + leechers)
  weight = 0                                              if seeders = 0 or leechers_ratio = 0
           leechers_ratio × 100 × leechers_ratio × leechers   otherwise
  ```
  Before its first successful announce, a torrent has a weight of 0.
- **Torrent rate** = global rate × weight / Σ weights (0 if Σ weights = 0). Recomputed on every seeders/leechers
  update, every torrent added/removed and every global rate redraw.
- **Accumulation**: on every tick, `uploaded += rate × actual elapsed time` (monotonic clock rather than an assumed
  interval). The counter starts from 0 each time the torrent is activated, as in Joal.

### 4.6 `library`

- **Folders**: `/data/torrents` (to seed) and `/data/torrents/archived`, created when needed.
- **Startup**: scan of `torrents/*.torrent`. Invalid files are reported in the logs and ignored.
- **Watcher** (`notify`, events grouped over 500 ms): any change sends `Rescan` to the engine.
  Safety net: periodic rescan every 30 s (network volumes where inotify reports nothing). The `archived` folder is
  ignored.
- **`add(bytes, filename)`** (upload from the UI):
  1. Parse first; if the file is invalid, it is rejected without writing anything.
  2. If its info_hash is already present in `torrents/`, it is rejected as a duplicate.
  3. Otherwise, atomic write (temporary file then `rename`) under a sanitized name.
- **`archive(infohash)`**: moves the file into `archived/`, with a `-1`, `-2`… suffix on name conflicts.
- **`delete(infohash)`**: deletes the file.

### 4.7 `engine`

**Torrent state**:
```rust
struct ActiveTorrent {
    torrent: MockedTorrent,
    status: Status,              // Starting | Seeding
    uploaded: u64,               // bytes accumulated since activation
    speed: u64,                  // assigned bytes/s
    seeders: Option<u32>, leechers: Option<u32>,
    interval: u32,               // last known interval, 5 s initially
    next_announce: Instant,
    next_event: Event,           // Started | None
    consecutive_fails: u8,
    last_error: Option<String>,
    tracker_index: usize,
}
```
Torrents present in the folder but not active form the **queue**.

**Rules** (identical to Joal):

| Situation | Action |
|---|---|
| Start | activate `simultaneousSeed` torrents from the queue; immediate `started` announce |
| `started`/regular OK | schedule the next one at `interval` s; update seeders/leechers and rates |
| OK with seeders < 1 or leechers < 1, and `keepTorrentWithZeroLeechers = false` | `stopped`, archive, activate the next one |
| Regular OK and `uploaded ≥ ratio × size` (if ratio ≠ -1) | `stopped`, archive, activate the next one |
| Failure | retry after the last known `interval` (5 s initially), with the same event |
| 5 consecutive failures | archive **without** `stopped`, activate the next one |
| File added to the folder | joins the queue; activated immediately if a slot is free |
| File removed / `DELETE` | `stopped` if active (without waiting for the answer), removal, activate the next one |
| Global stop | `stopped` for every active torrent, except those whose `started` has not succeeded yet; wait 5 s max |
| `PUT /api/config` | save; if seeding: global Stop, then Start with the new config |
| SIGTERM / Ctrl-C | global Stop, then exit |

**Snapshot** (published through `watch`, serialized as JSON for SSE):
```json
{
  "running": true,
  "client": "qbittorrent-5.2.4.client",
  "globalSpeed": 142000,
  "sessionUploaded": 3800000000,
  "queued": 0,
  "config": { "...": "AppConfig" },
  "torrents": [{
    "infoHash": "hex", "name": "…", "size": 0, "status": "seeding",
    "speed": 0, "uploaded": 0, "seeders": 12, "leechers": 5,
    "nextAnnounceIn": 1620, "consecutiveFails": 0, "lastError": null
  }]
}
```
`sessionUploaded` is the sum of `uploaded` since the process started, archived torrents included.

### 4.8 `api`

| Method | Route | Body / response |
|---|---|---|
| GET | `/api/events` | SSE `event: snapshot`, one on connection then every second; keep-alive 15 s |
| POST | `/api/seed/start` | 204 |
| POST | `/api/seed/stop` | 204 |
| GET | `/api/config` | `AppConfig` |
| PUT | `/api/config` | `AppConfig` → 200, or 400 `{ "field": "…", "message": "…" }` |
| GET | `/api/clients` | `["qbittorrent-5.2.4.client", …]` sorted by name then descending version |
| POST | `/api/torrents` | multipart, repeated `files` field → 200 `[{ "file", "status": "added"\|"duplicate"\|"invalid", "message"? }]`; 10 MB limit per file |
| DELETE | `/api/torrents/{infoHash}` | 204, or 404 |
| GET | `/*` | embedded `web/dist` files; falls back to `index.html` |

No CORS (same origin). In development, Vite proxies `/api` to the backend.

## 5. Interface

A single responsive page, with automatic light/dark theme (`prefers-color-scheme`). UI text is in French.

- **Header**: name, status badge (seeding / stopped / connection lost), emulated client, Start/Stop button,
  Settings button.
- **Metrics**: global rate, uploaded during the session, active / `simultaneousSeed`, queued.
- **Drop zone**: an always-visible box; while dragging, an overlay covers the whole page. The "browse" button opens
  a multi-file picker filtered on `.torrent`.
- **Notifications**: one per upload batch ("3 added, 1 duplicate"), plus network errors.
- **Torrent table**: name (truncated, full on hover), rate, uploaded, S/L, next announce (countdown refreshed by the
  snapshots, one per second), delete button (with confirmation). Torrents in error show "Failure n/5" in the
  warning colour, with `lastError` on hover. On mobile, the table becomes a list of cards.
- **Settings panel** (side panel): min/max fields (kB/s), simultaneous seeds, client picker with search, target
  ratio (disabled ↔ value), "keep at 0 leechers" switch. Save button; 400 errors are shown under the relevant field.
- **Resilience**: `EventSource` reconnects on its own; while disconnected, the badge switches to "connection lost".

`web/src` layout: `App.svelte`, `lib/api.ts` (fetch + EventSource), `lib/store.svelte.ts` (rune-based state),
`lib/format.ts` (bytes, rates, durations), `components/{Header,Metrics,DropZone,TorrentTable,SettingsPanel,Toasts}.svelte`.

## 6. Error handling

- Invalid `.torrent` file: rejected on upload with a message; ignored with a warning if dropped into the folder.
- Invalid `.client` profile: ignored with a warning. If the configured profile is invalid or missing, seeding
  refuses to start and the UI shows it.
- Unreadable `config.json` at startup: exit with an explicit message (a user's file is never overwritten).
- Network or tracker error: attached to the torrent (`lastError`), handled by the retry rules.
- No `unwrap()` on external data; an error on one torrent never affects the others.
- `tracing` logs on stdout (`docker logs`), level `RUST_LOG`.

## 7. Tests

**Rust (`cargo test`)**
- `bencode` / `torrent`: single-file, multi-file, `announce-list`, info_hash checked against a real test `.torrent`,
  non-canonical `info` dict.
- `emulation`:
  - URL encoder (exclusions, casing).
  - Each algorithm (format, length, no leading zeros, pool checksum).
  - Each refresh policy, with an injectable clock.
  - `build_query`: none/started/stopped events, numwant/numwantOnStop, `{ip}` removal, duplicate `&`.
  - Vectors taken from Joal's Java tests where they exist.
  - Global test: **all 95 profiles load and produce a query with no placeholder left**.
- `bandwidth`: weight formula, split, Σ weights = 0 case, accumulation.
- `library`: add, duplicate, invalid, archiving with a name conflict.
- **Engine + fake tracker integration** (local axum server):
  - Start → `started` received with the right parameters.
  - Regular announces with a growing `uploaded`.
  - Ratio reached → `stopped` + file in `archived/`.
  - 5 failures → archived without `stopped`.
  - Global stop → `stopped` for every torrent.
- **API**: multipart upload (added / duplicate / invalid), config validation, SSE receives a snapshot.

**Frontend**: `svelte-check` (types) and `vite build` without errors; manual check in the browser (drag and drop,
settings, deletion, mobile).

**Real-world check**: one announce to a public HTTP tracker with a free `.torrent` (a Linux distribution), inspecting
the request sent.

## 8. Delivery

- Multi-stage **Dockerfile**:
  1. `node:24-alpine` → `npm ci && npm run build` (web/dist).
  2. `rust:1-alpine` → `cargo build --release` (native musl target, static binary, `strip`, `lto`,
     `opt-level = "z"` to be weighed against `3`).
  3. `scratch` + CA certificates → `ratiod`, `EXPOSE 5081 49200`, `VOLUME /data`.
- Example **docker-compose.yml**: `./data:/data` volume, ports `5081` and `49200`, `restart: unless-stopped`.
- **README**: overview, Docker/compose launch, environment variables, migration from Joal (copy the contents of
  `joal-conf` into `data/`), usage disclaimer taken from Joal, credits.
- Apache 2.0 license with a `NOTICE` crediting Joal (Anthony Raymond) and the origin of the `.client` profiles.
- **Private** GitHub repository `xavierl/ratiod`.

## 9. Windows desktop mode (added 2026-10-15)

**Request**: on Windows, `ratiod.exe` must be a simple "portable" application: no terminal window, and launching
the exe shows the interface directly, like a desktop application.

**Approved choices**: native window (not a browser tab); closing the window minimizes it to the notification area,
seeding continues.

- **GUI subsystem**: `#![cfg_attr(windows, windows_subsystem = "windows")]`; no console ever appears, even on error.
- **Window**: `tao` + `wry` (WebView2, present on Windows 10/11), title "ratiod", ~1100×760, minimum size 420×500,
  ratiod icon. It loads the interface served by the internal server (`http://127.0.0.1:<port>`), which starts
  before the window. Dragging and dropping `.torrent` files must work in the window.
- **Notification area** (`tray-icon`): ratiod icon, "ratiod" tooltip; left click → shows the window and brings it to
  the front; menu « Ouvrir ratiod » (open) / « Quitter » (quit). « Quitter » shuts down cleanly (`stopped`
  announces, 5 s max wait like Stop) then closes the application. Closing the window only hides it.
- **Single instance**: if the interface port is already taken, the exe sends `POST /api/desktop/show` to
  `127.0.0.1:<port>`; a 204 answer means a ratiod is already running: its window comes to the front and the new
  instance exits without showing anything. Otherwise: a "port used by another program" message box.
  The `/api/desktop/show` route only exists (204) in desktop mode; elsewhere it returns 404.
- **Portable — data folder** (when `RATIOD_DATA_DIR` is not set), in order:
  1. `<exe folder>\data` if it exists;
  2. `%APPDATA%\ratiod` if it exists (v0.1.0 installations: no forced migration);
  3. `<exe folder>\data` if the exe folder is writable;
  4. `%APPDATA%\ratiod` otherwise.
- **Logs**: on Windows (desktop or `--headless`), written to `<data>\ratiod.log` (recreated on every launch), since
  there is no console.
- **Fatal errors** (WebView2 missing, port taken, data folder unreachable, unreadable `config.json`): a Windows
  message box in French, then exit. When WebView2 is missing, the message gives Microsoft's official download link.
- **Server mode**: `ratiod.exe --headless` keeps the server behaviour (no window, no icon); `RATIOD_OPEN_BROWSER`
  only applies to this mode. Linux and Docker are unchanged: desktop dependencies are only compiled for Windows.
- **Icon**: drawn in code (green `#059669` rounded square, white up arrow, like the favicon), used for the window,
  the notification area and the exe icon (generated as `.ico` by `build.rs`, embedded via `winresource`). No binary
  image file in the repository.
- **Tests**: icon drawing (size, transparent corners), data folder choice, `/api/desktop/show` route (204 + callback
  invoked / 404), detection of a running instance (local fake server). Manual check: exe in the GUI subsystem,
  window shown, drag and drop, hide on close, Quit, double launch.
