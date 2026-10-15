# ratiod

A lightweight BitTorrent ratio emulator written in Rust: a fake BitTorrent client that reports
simulated upload to trackers for your `.torrent` files, without transferring anything. A single binary of a few
MB with its web interface, available for **Windows**, **Linux** and **Docker**. No authentication: the Windows
and Linux packages only listen on `127.0.0.1`.

> **Disclaimer**: ratiod is not designed to help you download illegal content.
> Respect the law of your country and the rules of your trackers.

## Preview

![ratiod dashboard](docs/screenshots/dashboard-light.png)

<table>
  <tr>
    <td width="62%"><img src="docs/screenshots/dashboard-dark.png" alt="Dashboard in dark theme"></td>
    <td><img src="docs/screenshots/mobile.png" alt="Mobile layout"></td>
  </tr>
  <tr>
    <td align="center"><sub>Dark theme (follows the system setting)</sub></td>
    <td align="center"><sub>On mobile, the list switches to cards</sub></td>
  </tr>
</table>

![Settings panel](docs/screenshots/settings.png)
<p align="center"><sub>Settings: simulated rate, number of seeded torrents, emulated client (95 profiles), target ratio</sub></p>

The interface itself is in French.

## Installation

There are three ways to run ratiod. Both packages (Windows and Linux) only listen on the machine itself: the
interface is at **http://127.0.0.1:5081** and cannot be reached from the network.

Packages are on the [**Releases**](https://github.com/xavierl/ratiod/releases/latest) page. `SHA256SUMS.txt`
lets you verify the files.

| Package | System |
|---|---|
| `ratiod-<version>-windows-x86_64.zip` | Windows 10 / 11, 64-bit |
| `ratiod-<version>-linux-x86_64.tar.gz` | 64-bit Linux (PCs, most NAS devices) |
| `ratiod-<version>-linux-aarch64.tar.gz` | 64-bit ARM Linux (Raspberry Pi 4/5, ARM NAS devices) |

### Windows

1. Download `ratiod-<version>-windows-x86_64.zip` and extract it wherever you like (for example
   `Documents\ratiod`).
2. Double-click `ratiod.exe`: the interface opens directly in its own window, with no console and no browser.
   **Closing the window only hides it**: ratiod keeps seeding and its icon stays in the notification area (next
   to the clock; click `^` if it is hidden). Left-clicking the icon reopens the window; right-clicking offers
   « Ouvrir ratiod » (open) and « Quitter » (quit). **« Quitter » shuts ratiod down cleanly** (`stopped`
   announces to the trackers, 5 s at most). Launching `ratiod.exe` again while it is running simply brings its
   window to the front.
3. Windows 10/11 normally ships **Microsoft WebView2**, which renders the window. If it is missing, ratiod says
   so and gives the download link: https://go.microsoft.com/fwlink/p/?LinkId=2124703

On first launch:
- **SmartScreen** ("Windows protected your PC"): the executable is not signed. Click "More info", then
  "Run anyway".
- **Windows Firewall** asks you to allow ratiod on the network. This is the peer port announced to trackers;
  allow at least private networks. The web interface itself stays limited to `127.0.0.1`.

ratiod is **portable**: its data lives in the `data` folder created next to `ratiod.exe` (move the whole folder
to move the installation). A v0.1.0 installation keeps its `%APPDATA%\ratiod` folder as long as there is no
`data` folder next to the exe; if the exe's folder is not writable (for example `Program Files`),
`%APPDATA%\ratiod` is used. Logs are in the data folder (`data\ratiod.log` next to the exe, or
`%APPDATA%\ratiod\ratiod.log` under the rule above) and are recreated on every launch, since there is no console
any more.

`ratiod.exe --headless` keeps the old server behaviour, with no window and no icon (the interface then opens in
your browser, as before). It is meant for advanced use: there is no console, so no `Ctrl+C`; it only stops when
the process is ended (Task Manager), **without** `stopped` announces to the trackers.

To start ratiod when you log in: press `Win+R`, type `shell:startup`, and create a shortcut to `ratiod.exe` in
that folder.

### Linux

The binary is static: there is nothing else to install.

```bash
tar -xzf ratiod-<version>-linux-x86_64.tar.gz
cd ratiod-<version>-linux-x86_64
./ratiod
```

Then open http://127.0.0.1:5081 (`RATIOD_OPEN_BROWSER=true ./ratiod` opens it automatically). `Ctrl+C` stops
ratiod cleanly. Data lives in `~/.local/share/ratiod`.

**As a service, started with your session** (systemd, `ratiod.service` file included):

```bash
install -Dm755 ratiod ~/.local/bin/ratiod
install -Dm644 ratiod.service ~/.config/systemd/user/ratiod.service
systemctl --user daemon-reload
systemctl --user enable --now ratiod
journalctl --user -u ratiod -f          # logs
loginctl enable-linger "$USER"          # optional: keep running without an open session
```

To update, replace `~/.local/bin/ratiod`, then run `systemctl --user restart ratiod`.

### NAS or server (Docker Compose)

With Docker, the interface is deliberately exposed on the local network (`0.0.0.0:5081`) so it can be used from
another device. The image is built from source (no image is published); the first build takes a few minutes.

```bash
git clone https://github.com/xavierl/ratiod.git
cd ratiod
docker compose up -d --build
```

The interface is at `http://<nas-ip>:5081`. The `docker-compose.yml` file:

```yaml
services:
  ratiod:
    build: .
    image: ratiod:local
    container_name: ratiod
    restart: unless-stopped
    ports:
      - "5081:5081"     # web interface
      - "49200:49200"   # peer port announced to trackers
    volumes:
      - ./data:/data
    environment:
      RUST_LOG: info
      # RATIOD_AUTOSTART: "false"
    stop_grace_period: 15s
```

- **Web interface**: for another port, change the left-hand side (`"8080:5081"`). To restrict it to the NAS
  itself, write `"127.0.0.1:5081:5081"`. There is no authentication: do not expose it to the Internet.
- **Peer port**: the announced value is `RATIOD_PEER_PORT` (49200 in the image). To change it, edit both sides of
  the mapping **and** the variable, for example `"51413:51413"` and `RATIOD_PEER_PORT: "51413"`.
- **Data**: in `./data` next to the compose file.
- **Shutdown**: `stop_grace_period` leaves time to send the `stopped` announces to the trackers (5 s max).
- The image is based on `scratch`: there is no shell in the container; use `docker compose logs` to diagnose.

```bash
docker compose logs -f ratiod                 # follow the logs
docker compose down                           # stop (sends the "stopped" announces)
git pull && docker compose up -d --build      # update
```

## Usage

Drag your `.torrent` files onto the page, or drop them into the `torrents/` folder of the data folder (picked up
automatically). The data folder depends on the installation:

| Installation | Data folder |
|---|---|
| Windows | `data` next to `ratiod.exe` (or `%APPDATA%\ratiod`, see [Windows](#windows)) |
| Linux | `~/.local/share/ratiod` (or `$XDG_DATA_HOME/ratiod`) |
| Docker | `./data` next to `docker-compose.yml` |

## Reusing an existing configuration

ratiod reads the classic ratio-emulator layout. Copy an existing configuration folder into the data folder
(with ratiod stopped):

```
<data folder>/
├── config.json        ← used as is (keys: minUploadRate, maxUploadRate, simultaneousSeed, client, …)
├── clients/           ← optional: custom .client profiles (95 are built in)
└── torrents/
    └── archived/
```

## Configuration

Seeding settings (`config.json`) are edited from the interface. Environment variables:

| Variable | Default (packages) | Default (Docker) | Purpose |
|---|---|---|---|
| `RATIOD_BIND` | `127.0.0.1:5081` | `0.0.0.0:5081` | web interface address |
| `RATIOD_DATA_DIR` | see [Usage](#usage) | `/data` | data folder |
| `RATIOD_PEER_PORT` | random (49152–65534) | `49200` | port announced to trackers |
| `RATIOD_AUTOSTART` | `true` | `true` | start seeding on launch |
| `RATIOD_OPEN_BROWSER` | `true` on Windows, otherwise `false` | `false` | open the interface in the browser on launch (server / `--headless` mode only) |
| `RUST_LOG` | `info` | `info` | log level (`debug` to see every announce) |

On Windows, to change a variable: create a `ratiod.bat` file next to `ratiod.exe` containing, for example,
`set RATIOD_PEER_PORT=51413` and then `start "" ratiod.exe` on the next line, and run that file.

## How it works

- A global rate is drawn at random between min and max every 20 minutes, then split according to the number of
  leechers.
- A torrent is archived (in `torrents/archived/` of the data folder) when it reaches the target ratio, when it has
  no leechers left (if that option is disabled), or after 5 failed announces in a row.
- Requests reproduce exactly those of the chosen client (query, peer_id, key, headers).

## Development

```bash
cargo test                         # backend
cd web && npm install && npm test  # frontend
cd web && npm run dev              # UI on http://localhost:5173 (backend: cargo run)
```

**Publishing a release**: update `version` in `Cargo.toml`, then push a `vX.Y.Z` tag
(`git tag v0.2.0 && git push origin v0.2.0`), or run the **Release** workflow from GitHub's Actions tab with the
version (`v0.2.0`). The `.github/workflows/release.yml` workflow builds the interface, runs the tests, compiles
the Linux (x86_64, aarch64, static musl) and Windows (x86_64) binaries, then publishes the release with the
packages.

## License

Apache 2.0 — see `LICENSE` and `NOTICE`.
