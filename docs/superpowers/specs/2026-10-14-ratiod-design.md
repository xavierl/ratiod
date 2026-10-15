# ratiod — design

*Date : 2026-10-14 · Statut : en relecture*

Réécriture en Rust de [Joal](https://github.com/anthonyraymond/joal) (Apache 2.0) : un faux client BitTorrent qui
annonce aux trackers HTTP un upload fictif pour des `.torrent`, sans rien transférer.

## 1. Objectif et contraintes

**Ce qui a été demandé**
- Réécriture complète, bien plus légère et performante, en Rust.
- Frontend moderne et agréable, avec ajout de torrents directement depuis l'UI (glisser-déposer multiple).
- Aucun système d'authentification : usage local.
- Code hébergé dans un repo **privé** `xavierl/ratiod` sur GitHub.

**Contexte d'usage**
- Tourne en **Docker sur un NAS** ; l'UI est consultée depuis les appareils du LAN.
- Remplace une installation Joal existante : nouveau format libre, mais les `.client` et `.torrent` existants doivent
  rester réutilisables.
- Pas de CI ni de registre d'images : l'image est construite sur le NAS (`docker compose build`).

**Critères de réussite**
1. Les 95 profils `.client` de Joal se chargent et produisent une requête d'annonce valide.
2. Pour un même profil, la requête d'annonce (query + headers) a la même forme que celle de Joal.
3. Un dossier `joal-conf` existant (config.json, clients/, torrents/) fonctionne en le montant sur `/data`.
4. Image Docker < 15 Mo ; mémoire au repos < 30 Mo avec 200 torrents actifs.
5. Un `.torrent` glissé dans l'UI apparaît dans la liste et est annoncé en moins de 3 s s'il reste un slot libre.

## 2. Périmètre

**Inclus (parité Joal + ajouts validés)**
- Seed global start/stop, **démarrage automatique** au lancement (désactivable).
- Réglages : débit min/max, seeds simultanés, client émulé, ratio cible, garder les torrents à 0 leecher.
- Liste des torrents actifs avec débit, upload envoyé, seeders/leechers, prochaine annonce, erreurs.
- Ajout de `.torrent` par bouton et **glisser-déposer multiple** ; suppression d'un torrent.
- Surveillance du dossier `torrents/` (un fichier déposé directement sur le NAS est pris en compte).
- Archivage automatique (ratio atteint, 0 leecher, 5 échecs consécutifs).

**Exclu (YAGNI, ajoutable plus tard sans casser l'existant)**
- Trackers UDP, liens magnet, DHT.
- Pause/reprise ou ratio par torrent.
- Statistiques persistantes, historiques, graphiques.
- Authentification, HTTPS (usage LAN).
- Détection de l'IP publique (voir §4.4).
- CI, publication d'images.

## 3. Architecture

Un seul binaire Rust, `ratiod`, qui embarque le frontend compilé. Stack : `tokio`, `axum`, `hyper` + `hyper-rustls` (client HTTP sans headers implicites,
gzip via `flate2`), `serde`/`serde_json`, `notify`, `rust-embed`, `rand`/`rand_regex`, `sha1`, `tracing`. Frontend :
Svelte 5 (runes) + Vite + Tailwind CSS v4, sans routeur.

```
ratiod/
├── Cargo.toml
├── src/
│   ├── main.rs          démarrage, env, signaux, câblage
│   ├── config.rs        AppConfig (config.json) + Settings (variables d'env)
│   ├── torrent.rs       bencode + MockedTorrent (info_hash, nom, taille, trackers)
│   ├── bencode.rs       décodeur bencode minimal (avec plage d'octets brute du dict `info`)
│   ├── emulation/       profils .client
│   │   ├── mod.rs       ClientProfile : chargement, validation, build_query(), headers()
│   │   ├── algorithm.rs algorithmes peer_id / key
│   │   ├── generator.rs politiques de rafraîchissement
│   │   └── urlencode.rs encodeur d'URL configurable
│   ├── tracker.rs       annonce HTTP, parsing de la réponse, bascule de tracker
│   ├── bandwidth.rs     débit global aléatoire + répartition par poids + cumul
│   ├── library.rs       dossier torrents/ + archived/ + watcher
│   ├── engine.rs        acteur orchestrateur (seul propriétaire de l'état)
│   └── api.rs           routes axum, SSE, fichiers statiques embarqués
├── clients/             les ~95 profils .client de Joal (embarqués)
├── web/                 app Svelte (build → web/dist, embarqué)
├── Dockerfile
└── docker-compose.yml
```

### 3.1 Modèle de concurrence

L'`engine` est **une tâche tokio unique qui possède tout l'état** (torrents actifs, file d'attente, bande passante,
profil client). Les autres composants communiquent avec elle par messages :

- `mpsc<Command>` vers l'engine : `Start`, `Stop`, `UpdateConfig`, `Rescan`, `Delete(infohash)`,
  `AnnounceDone(infohash, Result)`, `Shutdown`. Un `Rescan` relit le dossier et réconcilie : nouveaux fichiers en file, fichiers disparus retirés (avec `stopped`).
- `watch<Snapshot>` depuis l'engine : l'état complet, sérialisable, publié à chaque changement et à chaque tick.

Les annonces HTTP sont lancées dans des tâches séparées (`tokio::spawn`), plafonnées à 20 simultanées par un
`Semaphore`. Leur résultat revient sous forme de `Command::AnnounceDone`. L'engine ne bloque jamais sur le réseau et
n'utilise aucun verrou.

La boucle de l'engine utilise `tokio::select!` sur : les commandes, un tick d'une seconde (cumul de l'upload,
déclenchement des annonces dont l'heure est passée) et le rafraîchissement du débit global (20 min).

### 3.2 Flux de données

```
UI ──POST /api/torrents──▶ api ──écrit──▶ /data/torrents/x.torrent
                                              │ notify
                                              ▼
                         library ──Rescan──▶ engine ──spawn──▶ tracker (started)
                                                         ▲                    │
                                                         └──AnnounceDone──────┘
engine ──watch<Snapshot>──▶ api ──SSE /api/events──▶ UI
```

Un upload depuis l'UI passe par le même chemin qu'un fichier déposé à la main : une seule source de vérité,
le dossier.

## 4. Composants

### 4.1 `config`

**`/data/config.json`** : mêmes clés que Joal, relu au démarrage, réécrit par `PUT /api/config`. S'il est absent,
il est créé avec les valeurs ci-dessous.

| Clé | Type | Défaut | Validation |
|---|---|---|---|
| `minUploadRate` | u64 (ko/s) | 30 | ≥ 0 |
| `maxUploadRate` | u64 (ko/s) | 170 | ≥ `minUploadRate` |
| `simultaneousSeed` | u32 | 200 | ≥ 1 |
| `client` | string | dernier profil qBittorrent embarqué | doit exister |
| `keepTorrentWithZeroLeechers` | bool | true | — |
| `uploadRatioTarget` | f32 | -1 | `-1` (désactivé) ou > 0 |

Les valeurs par défaut sont celles de Joal. Comme dans Joal, 1 ko = 1000 octets.

**Variables d'environnement** (non modifiables depuis l'UI) :

| Variable | Défaut binaire | Défaut image Docker |
|---|---|---|
| `RATIOD_BIND` | `127.0.0.1:5081` | `0.0.0.0:5081` |
| `RATIOD_DATA_DIR` | `./data` | `/data` |
| `RATIOD_PEER_PORT` | aléatoire 49152–65534 | `49200` |
| `RATIOD_AUTOSTART` | `true` | `true` |
| `RUST_LOG` | `info` | `info` |

### 4.2 `torrent` et `bencode`

- Décodeur bencode maison (≈150 lignes) qui conserve la plage d'octets brute du dict `info` pour calculer
  `info_hash = SHA-1(octets bruts)`. Pas de ré-encodage, donc pas de risque de hash faux sur un torrent non canonique.
- `MockedTorrent { info_hash: [u8; 20], name, size, trackers: Vec<Url> }`.
  - `size` : `length`, ou somme des `files[].length`.
  - `trackers` : `announce-list` aplatie dans l'ordre, sinon `announce`.
  - Seuls les trackers `http`/`https` sont conservés ; un torrent sans aucun tracker HTTP est refusé.

### 4.3 `emulation`

Charge les profils **au format `.client` de Joal sans modification**. Les profils embarqués (`clients/`, via
`rust-embed`) sont fusionnés avec `/data/clients/*.client` : à nom égal, le fichier du volume gagne.

**Algorithmes** (champ `algorithm.type`) :

| Type | Utilisé pour | Comportement |
|---|---|---|
| `REGEX` | peer_id, key | chaîne aléatoire conforme à `pattern` (`rand_regex`) |
| `RANDOM_POOL_WITH_CHECKSUM` | peer_id | `prefix` complété à 20 caractères : chaque caractère est `charactersPool[octet aléatoire % base]`, le dernier rend la somme des indices multiple de `base` (même calcul que `RandomPoolWithChecksumPeerIdAlgorithm.java`) |
| `HASH` | key | `length` caractères hex majuscules aléatoires |
| `HASH_NO_LEADING_ZERO` | key | idem, zéros de tête retirés |
| `DIGIT_RANGE_TRANSFORMED_TO_HEX_WITHOUT_LEADING_ZEROES` | key | entier dans `[inclusiveLowerBound, inclusiveUpperBound]` → hex sans zéros de tête |

`keyCase` (`upper`/`lower`/`none`) s'applique à la key générée.

**Politiques de rafraîchissement** (`refreshOn`) :

| Valeur | Comportement |
|---|---|
| `NEVER` | une valeur pour toute la durée du processus |
| `ALWAYS` | nouvelle valeur à chaque annonce |
| `TIMED` | nouvelle valeur toutes les `refreshEvery` secondes |
| `TORRENT_PERSISTENT` | une valeur par torrent, conservée jusqu'à son retrait |
| `TORRENT_VOLATILE` | une valeur par torrent, oubliée après l'annonce `stopped` |
| `TIMED_OR_AFTER_STARTED_ANNOUNCE` | nouvelle valeur après `refreshEvery` secondes, ou après chaque annonce `started` |

**Encodeur d'URL** : chaque octet est conservé s'il correspond à `encodingExclusionPattern`, sinon encodé `%XX`
avec la casse `encodedHexCase`. Il s'applique à l'info_hash (20 octets bruts), à la key, et au peer_id si
`shouldUrlEncode` est vrai.

**Construction de la query** (même algorithme que `BitTorrentClient.createRequestQuery`) :
1. Fusion des `&` consécutifs du template.
2. Remplacement de `{infohash} {uploaded} {downloaded} {left} {port} {numwant} {peerid} {key}`.
3. Suppression des paramètres `…={ip}` / `…={ipv6}`.
4. Événement `none` : suppression du paramètre `…={event}` ; sinon `started` / `stopped`.
5. `numwant` vaut `numwantOnStop` pour `stopped`, `numwant` sinon.
6. Si un `{…}` subsiste, **le profil est invalide** (refusé au chargement, testé sur des valeurs factices).
7. Suppression des `&` en début et en fin.

**Headers** : liste ordonnée du profil, avec `{java}` → `21.0.4`, `{os}` → `Linux`, `{locale}` → `en-US`
(valeurs fixes et crédibles). Un placeholder inconnu rend le profil invalide.

### 4.4 `tracker`

- Requête `GET {announce_url}{?|&}{query}`. La query est ajoutée **telle quelle, sans ré-encodage** : l'URL est
  construite comme chaîne puis parsée, et un test vérifie que le chemin+query envoyé est identique octet pour octet.
- Client `hyper` (et non `reqwest`, qui ajoute un `Accept: */*` impossible à retirer) : seuls `Host` (en premier, comme les vrais clients) puis les headers du profil sont envoyés, en casse « Title-Case ».
  Timeout de 30 s, gzip accepté si le profil l'annonce.
- Réponse bencode :
  - `failure reason` → erreur.
  - Sinon `interval` (obligatoire), `complete`, `incomplete` (0 si absents).
  - `seeders = max(0, complete − 1)` (on se retire du compte), `leechers = incomplete`.
- En cas d'échec, on passe au tracker suivant de la liste (en boucle). Le compteur d'échecs consécutifs est remis à
  zéro au premier succès.
- **IP publique** : non détectée. Aucun profil n'utilise `{ip}`, et Joal retire déjà `{ipv6}` quand l'IP détectée
  est une IPv4 (cas habituel). Cela évite des appels à des services tiers.
- **Port annoncé** : `RATIOD_PEER_PORT`. Un `TcpListener` y écoute réellement (connexions acceptées puis fermées)
  pour que le port paraisse ouvert.

### 4.5 `bandwidth`

Reprise exacte de `BandwidthDispatcher` :
- **Débit global** tiré uniformément dans `[min, max)` ko/s (ou `max` si min = max), renouvelé toutes les 20 min et
  à chaque changement de config.
- **Poids d'un torrent** :
  ```
  ratio_leechers = leechers / (seeders + leechers)
  poids = 0                                         si seeders = 0 ou ratio_leechers = 0
          ratio_leechers × 100 × ratio_leechers × leechers   sinon
  ```
  Avant sa première annonce réussie, un torrent a un poids de 0.
- **Débit d'un torrent** = débit global × poids / Σ poids (0 si Σ poids = 0). Recalculé à chaque mise à jour des
  seeders/leechers, à chaque ajout/retrait de torrent et à chaque renouvellement du débit global.
- **Cumul** : à chaque tick, `uploaded += débit × durée réelle écoulée` (horloge monotone plutôt qu'un intervalle
  supposé). Le compteur part de 0 à chaque activation du torrent, comme dans Joal.

### 4.6 `library`

- **Dossiers** : `/data/torrents` (à seeder) et `/data/torrents/archived`, créés au besoin.
- **Démarrage** : scan de `torrents/*.torrent`. Les fichiers invalides sont signalés dans les logs et ignorés.
- **Watcher** (`notify`, regroupement des événements sur 500 ms) : tout changement envoie `Rescan` à l'engine.
  Filet de sécurité : rescan périodique toutes les 30 s (volumes réseau où inotify ne remonte rien). Le dossier `archived` est ignoré.
- **`add(bytes, filename)`** (upload depuis l'UI) :
  1. Parse d'abord ; si le fichier est invalide, il est refusé sans rien écrire.
  2. Si son info_hash est déjà présent dans `torrents/`, il est refusé comme doublon.
  3. Sinon, écriture atomique (fichier temporaire puis `rename`) sous un nom assaini.
- **`archive(infohash)`** : déplace le fichier dans `archived/`, avec suffixe `-1`, `-2`… en cas de conflit de nom.
- **`delete(infohash)`** : supprime le fichier.

### 4.7 `engine`

**État d'un torrent** :
```rust
struct ActiveTorrent {
    torrent: MockedTorrent,
    status: Status,              // Starting | Seeding
    uploaded: u64,               // octets cumulés depuis l'activation
    speed: u64,                  // octets/s attribués
    seeders: Option<u32>, leechers: Option<u32>,
    interval: u32,               // dernier interval connu, 5 s au départ
    next_announce: Instant,
    next_event: Event,           // Started | None
    consecutive_fails: u8,
    last_error: Option<String>,
    tracker_index: usize,
}
```
Les torrents présents dans le dossier mais non actifs forment la **file d'attente**.

**Règles** (identiques à Joal) :

| Situation | Action |
|---|---|
| Start | active `simultaneousSeed` torrents depuis la file ; annonce `started` immédiate |
| `started`/régulière OK | programme la suivante à `interval` s ; met à jour seeders/leechers et les débits |
| OK avec seeders < 1 ou leechers < 1, et `keepTorrentWithZeroLeechers = false` | `stopped`, archive, active le suivant |
| Régulière OK et `uploaded ≥ ratio × taille` (si ratio ≠ -1) | `stopped`, archive, active le suivant |
| Échec | réessai après le dernier `interval` connu (5 s au départ), avec le même événement |
| 5 échecs consécutifs | archive **sans** `stopped`, active le suivant |
| Fichier ajouté au dossier | rejoint la file ; activé immédiatement si un slot est libre |
| Fichier supprimé / `DELETE` | `stopped` s'il est actif (sans attendre la réponse), retrait, active le suivant |
| Stop global | `stopped` pour tous les actifs, sauf ceux dont le `started` n'a pas encore réussi ; attente max 5 s |
| `PUT /api/config` | enregistre ; si le seed tourne : Stop global puis Start avec la nouvelle config |
| SIGTERM / Ctrl-C | Stop global puis sortie |

**Snapshot** (publié via `watch`, sérialisé en JSON pour le SSE) :
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
`sessionUploaded` est la somme des `uploaded` depuis le lancement du processus, torrents archivés compris.

### 4.8 `api`

| Méthode | Route | Corps / réponse |
|---|---|---|
| GET | `/api/events` | SSE `event: snapshot`, un à la connexion puis toutes les secondes ; keep-alive 15 s |
| POST | `/api/seed/start` | 204 |
| POST | `/api/seed/stop` | 204 |
| GET | `/api/config` | `AppConfig` |
| PUT | `/api/config` | `AppConfig` → 200, ou 400 `{ "field": "…", "message": "…" }` |
| GET | `/api/clients` | `["qbittorrent-5.2.4.client", …]` triés par nom puis version décroissante |
| POST | `/api/torrents` | multipart, champ `files` répété → 200 `[{ "file", "status": "added"\|"duplicate"\|"invalid", "message"? }]` ; limite 10 Mo par fichier |
| DELETE | `/api/torrents/{infoHash}` | 204, ou 404 |
| GET | `/*` | fichiers de `web/dist` embarqués ; repli sur `index.html` |

Pas de CORS (même origine). En développement, Vite redirige `/api` vers le backend.

## 5. Interface

Une page unique, responsive, avec thème clair/sombre automatique (`prefers-color-scheme`).

- **En-tête** : nom, pastille d'état (Seed actif / Arrêté / Connexion perdue), client émulé, bouton Start/Stop,
  bouton Réglages.
- **Métriques** : débit global, envoyé pendant la session, actifs / `simultaneousSeed`, en file.
- **Zone de dépôt** : encart visible en permanence ; pendant un glisser, un calque couvre toute la page.
  Le bouton « parcourir » ouvre un sélecteur multi-fichiers filtré sur `.torrent`.
- **Notifications** : une par lot d'upload (« 3 ajoutés, 1 doublon »), plus les erreurs réseau.
- **Tableau des torrents** : nom (tronqué, complet au survol), débit, envoyé, S/L, prochaine annonce (compte à
  rebours rafraîchi par les snapshots, un par seconde), bouton supprimer (avec confirmation). Les torrents en erreur affichent
  « Échec n/5 » en couleur d'avertissement, avec `lastError` au survol. Sur mobile, le tableau devient une liste de
  cartes.
- **Panneau Réglages** (latéral) : deux champs min/max (ko/s), seeds simultanés, sélecteur de client avec
  recherche, ratio cible (désactivé ↔ valeur), interrupteur « garder à 0 leecher ». Bouton Enregistrer ; les
  erreurs 400 s'affichent sous le champ concerné.
- **Robustesse** : `EventSource` se reconnecte seul ; pendant la coupure, la pastille passe à « Connexion perdue ».

Arborescence `web/src` : `App.svelte`, `lib/api.ts` (fetch + EventSource), `lib/store.svelte.ts` (état en runes),
`lib/format.ts` (octets, débits, durées), `components/{Header,Metrics,DropZone,TorrentTable,SettingsPanel,Toasts}.svelte`.

## 6. Gestion des erreurs

- Fichier `.torrent` invalide : refusé à l'upload avec message ; ignoré avec warning s'il est déposé dans le dossier.
- Profil `.client` invalide : ignoré avec warning. Si le profil configuré est invalide ou absent, le seed refuse de
  démarrer et l'UI l'affiche.
- `config.json` illisible au démarrage : sortie avec un message explicite (on n'écrase pas un fichier de
  l'utilisateur).
- Erreur réseau ou tracker : attachée au torrent (`lastError`), gérée par les règles de réessai.
- Aucun `unwrap()` sur une donnée externe ; une erreur sur un torrent n'affecte jamais les autres.
- Logs `tracing` sur stdout (`docker logs`), niveau `RUST_LOG`.

## 7. Tests

**Rust (`cargo test`)**
- `bencode` / `torrent` : mono-fichier, multi-fichiers, `announce-list`, info_hash vérifié sur un vrai `.torrent` de
  test, dict `info` non canonique.
- `emulation` :
  - Encodeur d'URL (exclusions, casse).
  - Chaque algorithme (format, longueur, absence de zéros de tête, somme de contrôle du pool).
  - Chaque politique de rafraîchissement, avec une horloge injectable.
  - `build_query` : events none/started/stopped, numwant/numwantOnStop, suppression de `{ip}`, `&` en double.
  - Vecteurs repris des tests Java de Joal quand ils existent.
  - Test global : **les 95 profils se chargent et produisent une query sans placeholder**.
- `bandwidth` : formule de poids, répartition, cas Σ poids = 0, cumul.
- `library` : ajout, doublon, invalide, archivage avec conflit de nom.
- **Intégration engine + faux tracker** (serveur axum local) :
  - Start → `started` reçu avec les bons paramètres.
  - Annonces régulières avec `uploaded` croissant.
  - Ratio atteint → `stopped` + fichier dans `archived/`.
  - 5 échecs → archivé sans `stopped`.
  - Stop global → `stopped` pour chaque torrent.
- **API** : upload multipart (ajouté / doublon / invalide), validation de config, SSE reçoit un snapshot.

**Frontend** : `svelte-check` (types) et `vite build` sans erreur ; vérification manuelle dans le navigateur
(glisser-déposer, réglages, suppression, mobile).

**Vérification réelle** : une annonce vers un tracker public HTTP avec un `.torrent` libre (distribution Linux), en
inspectant la requête envoyée.

## 8. Livraison

- **Dockerfile** multi-stage :
  1. `node:24-alpine` → `npm ci && npm run build` (web/dist).
  2. `rust:1-alpine` → `cargo build --release` (cible musl native, binaire statique, `strip`, `lto`,
     `opt-level = "z"` à évaluer contre `3`).
  3. `scratch` + certificats CA → `ratiod`, `EXPOSE 5081 49200`, `VOLUME /data`.
- **docker-compose.yml** d'exemple : volume `./data:/data`, ports `5081` et `49200`, `restart: unless-stopped`.
- **README** : présentation, lancement Docker/compose, variables d'environnement, migration depuis Joal (copier le
  contenu de `joal-conf` dans `data/`), avertissement d'usage repris de Joal, crédits.
- Licence Apache 2.0 avec `NOTICE` mentionnant Joal (Anthony Raymond) et l'origine des profils `.client`.
- Repo GitHub **privé** `xavierl/ratiod`.

## 9. Mode bureau Windows (ajout du 2026-10-15)

**Demande** : sous Windows, `ratiod.exe` doit être une application « portable » simple : aucune fenêtre de terminal,
et le lancement de l'exe affiche directement l'interface, comme une application de bureau.

**Choix validés** : fenêtre native (pas d'onglet de navigateur) ; fermer la fenêtre la réduit dans la zone de
notification, le seed continue.

- **Sous-système GUI** : `#![cfg_attr(windows, windows_subsystem = "windows")]` ; aucune console n'apparaît, même en cas
  d'erreur.
- **Fenêtre** : `tao` + `wry` (WebView2, présent sur Windows 10/11), titre « ratiod », ~1100×760, taille minimale
  420×500, icône ratiod. Elle charge l'interface servie par le serveur interne (`http://127.0.0.1:<port>`), qui démarre
  avant la fenêtre. Le glisser-déposer de `.torrent` doit fonctionner dans la fenêtre.
- **Zone de notification** (`tray-icon`) : icône ratiod, info-bulle « ratiod » ; clic gauche → affiche et met au premier
  plan la fenêtre ; menu « Ouvrir ratiod » / « Quitter ». « Quitter » arrête proprement (annonces `stopped`, attente
  max 5 s comme le Stop) puis ferme l'application. Fermer la fenêtre la masque seulement.
- **Instance unique** : si le port de l'interface est déjà occupé, l'exe envoie `POST /api/desktop/show` à
  `127.0.0.1:<port>` ; une réponse 204 signifie qu'un ratiod tourne déjà : sa fenêtre revient au premier plan et la
  nouvelle instance quitte sans rien afficher. Sinon : boîte de message « port occupé par un autre programme ».
  La route `/api/desktop/show` n'existe (204) qu'en mode bureau ; ailleurs 404.
- **Portable — dossier de données** (si `RATIOD_DATA_DIR` n'est pas défini), dans l'ordre :
  1. `<dossier de l'exe>\data` s'il existe ;
  2. `%APPDATA%\ratiod` s'il existe (installations v0.1.0 : pas de migration forcée) ;
  3. `<dossier de l'exe>\data` si le dossier de l'exe est modifiable ;
  4. `%APPDATA%\ratiod` sinon.
- **Logs** : sous Windows (bureau ou `--headless`), écrits dans `<data>\ratiod.log` (recréé à chaque lancement), puisqu'il
  n'y a pas de console.
- **Erreurs bloquantes** (WebView2 absent, port occupé, dossier de données inaccessible, `config.json` illisible) :
  boîte de message Windows en français, puis sortie. Pour WebView2 absent, le message donne le lien de téléchargement
  officiel de Microsoft.
- **Mode serveur** : `ratiod.exe --headless` garde le comportement serveur (sans fenêtre ni icône) ;
  `RATIOD_OPEN_BROWSER` ne s'applique qu'à ce mode. Linux et Docker sont inchangés : les dépendances de bureau ne sont
  compilées que pour Windows.
- **Icône** : dessinée par le code (carré arrondi vert `#059669`, flèche blanche vers le haut, comme le favicon), utilisée
  pour la fenêtre, la zone de notification et l'icône de l'exe (générée en `.ico` par `build.rs`, intégrée via
  `winresource`). Aucun fichier image binaire dans le repo.
- **Tests** : dessin de l'icône (taille, transparence des coins), choix du dossier de données, route
  `/api/desktop/show` (204 + rappel appelé / 404), détection d'une instance existante (faux serveur local). Vérification
  manuelle : exe en sous-système GUI, fenêtre affichée, glisser-déposer, masquage à la fermeture, Quitter, double
  lancement.
