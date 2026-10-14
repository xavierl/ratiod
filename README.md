# ratiod

Réécriture en Rust de [Joal](https://github.com/anthonyraymond/joal) : un faux client BitTorrent qui annonce
aux trackers un upload simulé pour tes fichiers `.torrent`, sans rien transférer. Un seul binaire de quelques Mo,
une interface web moderne, pas d'authentification (usage sur réseau local).

> **Avertissement** (repris de Joal) : ratiod n'est pas conçu pour aider à télécharger du contenu illégal.
> Respecte la loi de ton pays et les règles de tes trackers.

## Démarrage sur un NAS (Docker Compose)

Prérequis : Docker avec le plugin Compose (`docker compose version`). L'image est construite localement à partir
des sources (aucune image n'est publiée) ; la première construction prend quelques minutes.

```bash
git clone https://github.com/xavierl/ratiod.git   # dépôt privé : identifiant GitHub + token d'accès
cd ratiod
docker compose up -d --build
```

L'interface est sur `http://<ip-du-nas>:5081`. Glisse tes `.torrent` sur la page, ou dépose-les dans
`data/torrents/` (pris en compte automatiquement).

### Le fichier `docker-compose.yml`

```yaml
services:
  ratiod:
    build: .
    image: ratiod:local
    container_name: ratiod
    restart: unless-stopped
    ports:
      - "5081:5081"     # interface web
      - "49200:49200"   # port pair annoncé aux trackers
    volumes:
      - ./data:/data
    environment:
      RUST_LOG: info
      # RATIOD_AUTOSTART: "false"
    stop_grace_period: 15s
```

- **Interface web** : pour l'exposer sur un autre port, change seulement la partie gauche (`"8080:5081"`).
  Il n'y a pas d'authentification : ne l'ouvre pas sur Internet.
- **Port pair** : la valeur annoncée aux trackers est `RATIOD_PEER_PORT` (49200 dans l'image). Pour en changer,
  modifie les deux côtés du mapping **et** la variable, par exemple `"51413:51413"` et `RATIOD_PEER_PORT: "51413"`.
- **Données** : tout est dans `./data` à côté du fichier compose (`config.json`, `torrents/`, `clients/`).
- **Arrêt** : `stop_grace_period` laisse le temps d'envoyer les annonces `stopped` aux trackers (5 s max).
- L'image part de `scratch` : pas de shell dans le conteneur, utilise `docker compose logs` pour diagnostiquer.

### Commandes utiles

```bash
docker compose logs -f ratiod                 # suivre les logs
docker compose restart ratiod                 # redémarrer
docker compose down                           # arrêter (envoie les annonces « stopped »)
git pull && docker compose up -d --build      # mettre à jour vers la dernière version
```

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
