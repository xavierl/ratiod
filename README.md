# ratiod

Réécriture en Rust de [Joal](https://github.com/anthonyraymond/joal) : un faux client BitTorrent qui annonce
aux trackers un upload simulé pour tes fichiers `.torrent`, sans rien transférer. Un seul binaire de quelques Mo,
une interface web moderne, pas d'authentification (usage sur réseau local).

> **Avertissement** (repris de Joal) : ratiod n'est pas conçu pour aider à télécharger du contenu illégal.
> Respecte la loi de ton pays et les règles de tes trackers.

## Démarrage sur un NAS (Docker)

```bash
git clone https://github.com/xavierl/ratiod.git
cd ratiod
docker compose up -d --build
```

L'interface est sur `http://<ip-du-nas>:5081`. Glisse tes `.torrent` sur la page, ou dépose-les dans
`data/torrents/`.

## Migration depuis Joal

Copie le contenu de ton dossier `joal-conf` dans `data/` :

```
data/
├── config.json        ← repris tel quel (mêmes clés que Joal)
├── clients/           ← facultatif : profils .client perso (les 95 de Joal sont intégrés)
└── torrents/
    └── archived/
```

## Configuration

Les réglages de seed (`config.json`) se modifient depuis l'interface. Variables d'environnement :

| Variable | Défaut (image Docker) | Rôle |
|---|---|---|
| `RATIOD_BIND` | `0.0.0.0:5081` | adresse de l'interface web |
| `RATIOD_DATA_DIR` | `/data` | dossier de données |
| `RATIOD_PEER_PORT` | `49200` | port annoncé aux trackers (à publier dans Docker) |
| `RATIOD_AUTOSTART` | `true` | démarre le seed au lancement |
| `RUST_LOG` | `info` | niveau de logs (`debug` pour voir chaque annonce) |

Hors Docker, `RATIOD_BIND` vaut `127.0.0.1:5081` et `RATIOD_DATA_DIR` vaut `./data`.

## Fonctionnement

- Débit global tiré au hasard entre min et max toutes les 20 min, réparti selon le nombre de leechers.
- Un torrent est archivé (`data/torrents/archived/`) quand il atteint le ratio cible, quand il n'a plus de
  leecher (si l'option est désactivée) ou après 5 annonces ratées d'affilée.
- Les requêtes reproduisent exactement celles du client choisi (query, peer_id, key, headers).

## Développement

```bash
cargo test                         # backend
cd web && npm install && npm test  # frontend
cd web && npm run dev              # UI sur http://localhost:5173 (backend : cargo run)
```

## Licence

Apache 2.0 — voir `LICENSE` et `NOTICE`.
