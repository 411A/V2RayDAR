<p align="center">
  <a href="https://deepwiki.com/411A/V2RayDAR">
    <img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki About V2RayDAR">
  </a>
</p>

<p align="center">
  <strong>🌐 Available in</strong><br>
  <strong><a href="../README.md">English</a></strong>
  • <strong><a href="README.fa.md">فارسی</a></strong>
  • <strong><a href="README.zh-CN.md">简体中文</a></strong>
  • <strong><a href="README.ru.md">Русский</a></strong>
  • <strong><a href="README.fr.md">Français</a></strong>
</p>

<p align="center">
  <img src="../assets/V2RayDAR_logo_v1.png" alt="V2RayDAR logo" width="200" height="200">
</p>

<h1 align="center">V2RayDAR</h1>

<p align="center">
  <em>Détection et Reconnaissance V2Ray — se prononce comme <code>v2ray</code> + <code>radar</code>.</em>
</p>

<p align="center">
  Un outil CLI/TUI rapide en Rust qui récupère les sources d'abonnement V2Ray / Clash / Mihomo, les valide via votre réseau réel avec <code>sing-box</code>, classe les configurations fonctionnelles et les republie sur une URL d'abonnement locale pour vos clients v2rayN / v2rayNG / sing-box / Clash Verge / Mihomo.
</p>

<p align="center">
  📘 <a href="guide.md">Lire le guide développeur détaillé</a>
</p>

## 🖥️ Aperçu TUI Windows

<p align="center">
  <img src="../assets/Windows_TUI_v0.5.2.png" alt="Windows TUI" width="100%">
</p>

## 🤔 Pourquoi V2RayDAR

- Récupère les abonnements en parallèle depuis un nombre illimité de sources.
- Prend en charge les formats brut, base64, JSON et YAML — ainsi que les liens de partage `vmess`, `vless`, `trojan`, `ss`, `ssr`, `hysteria2`, `hy2`, `tuic`.
- **Parse les configs Clash/Mihomo YAML** — ajoutez une URL d'abonnement Mihomo et V2RayDAR extrait automatiquement toutes les entrées proxy.
- **Conversion bidirectionnelle** — convertit entre les liens de partage V2Ray et les entrées proxy Clash/Mihomo YAML.
- Valide chaque candidat via votre réseau réel avec `sing-box` (charge réellement une URL de test à travers le proxy).
- **Sortie double format** — sert les configs fonctionnelles en tant que liens de partage V2Ray (`/subscription`) **et** configs Mihomo YAML complètes (`/mihomo.yaml`).
- Republie les meilleures configs fonctionnelles sur une URL locale pour tout client compatible.
- Survit aux réseaux restreints via les configs précédemment testées en base de données, un config passerelle réseau ou `emergency_config`.
- Partage LAN optionnel avec protection par token, pour utiliser le même abonnement depuis votre téléphone.

## 📦 Installation rapide

Copiez la commande correspondant à votre OS dans un terminal. Le script d'installation détecte votre plateforme, télécharge la dernière version avec `sing-box` et configure tout. Le mode portable s'installe dans `Desktop/V2RayDAR` (si le dossier Bureau existe), sinon dans `~/V2RayDAR`. Le mode utilisateur installe le binaire dans `~/.local/bin`.

**Mode portable** (recommandé) — tout dans un dossier, lancement avec `--portable` :
```bash
# Linux
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
```

**Installation utilisateur** — binaire dans `~/.local/bin`, données dans le répertoire home :
```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh -s -- --user

# Windows
irm https://raw.githubusercontent.com/411A/V2RayDAR/main/install.ps1 | iex
# Puis choisissez l'option 2 quand demandé
```

**Android / Termux :**
```bash
# Installez sing-box, puis lancez le script d'installation
pkg update -y && pkg install -y curl tar sing-box=1.13.13
curl -fsSL https://raw.githubusercontent.com/411A/V2RayDAR/main/install.sh | sh
# Utilisez toujours --no-tui sur Termux (la souris ne fonctionne pas dans les terminaux Termux)
cd V2RayDAR && ./v2raydar --no-tui
```

**Téléchargement manuel** — téléchargez l'archive pour votre OS depuis [Releases](https://github.com/411A/V2RayDAR/releases/latest) et lancez avec `--portable`.

Le script d'installation vérifie les checksums SHA-256, détecte les installations existantes et propose une mise à jour (en préservant `configs.yaml`, `data.db` et `v2raydar_data/`), et ne nécessite pas sudo par défaut.

## 🔰 Démarrage rapide

Après l'installation avec le script ci-dessus, lancez `v2raydar` (ou `v2raydar.exe` sous Windows). Au premier lancement, un `configs.yaml` est créé avec des sources d'abonnement pré-sélectionnées.

1. **Attendez le remplissage.** L'application récupère vos sources d'abonnement en parallèle, teste chaque config via votre réseau et classe les configs fonctionnelles. L'endpoint est actif dès le début — votre client peut s'y connecter immédiatement.
2. **Pointez votre client** vers l'URL d'abonnement :

| Client | Endpoint |
| --- | --- |
| v2rayN / v2rayNG | `http://127.0.0.1:27141/subscription` (base64) |
| sing-box | `http://127.0.0.1:27141/subscription.txt` (brut) |
| Clash Verge / Mihomo | `http://127.0.0.1:27141/mihomo.yaml` |

3. **Contrôles TUI :**

| Touche | Action |
| --- | --- |
| `↑` / `↓` ou `j` / `k` | Navigation |
| `Enter` | Sélectionner / basculer / confirmer |
| `Esc` / `Ctrl+H` | Retour |
| `Space` | Activer/désactiver l'abonnement |
| `e` | Modifier l'abonnement sélectionné |
| `q` | Quitter |
| `:` | Mode commande — `:q` quitter, `:w` sauvegarder, `:a` ajouter, `:d` supprimer, `:n` renommer, `:u` URL, `:p` priorité |

4. **Modifier les paramètres** depuis le menu principal TUI (Configurations) ou en éditant directement `configs.yaml` — les changements prennent effet au prochain rafraîchissement. Paramètres clés : `top_n`, `refresh_seconds`, `sharing.enabled`, `probe.mode`.
5. **Quitter** avec `q` ou `:q`. L'endpoint s'arrête à la fermeture.

### Modes d'exécution

```bash
v2raydar                # TUI + endpoint d'abonnement local
v2raydar --no-tui       # sans interface — endpoint et logs uniquement
v2raydar --once         # un rafraîchissement, afficher les résultats, quitter
v2raydar --portable     # données à côté de l'exécutable
v2raydar --uninstall    # supprimer les données et règles de pare-feu
```

Les utilisateurs Windows remplacent `v2raydar` par `v2raydar.exe`. Sous macOS, ouvrez le `.app` une fois et Gatekeeper s'en souviendra.

## ⚙️ Configuration par défaut

<details>
  <summary>👣 <strong>configs.yaml</strong> — tableau de toutes les clés, valeurs par défaut et leur rôle. Explications complètes dans le <a href="guide.md">guide développeur</a>.</summary>

| Clé | Par défaut | Rôle |
| --- | --- | --- |
| `bind` | `127.0.0.1:27141` | Adresse HTTP locale pour `/subscription`, `/subscription.txt`, `/results` et `/health`. |
| `top_n` | `10` | Nombre de configs fonctionnelles publiées aux clients. |
| `refresh_seconds` | `300` | Intervalle de rafraîchissement automatique (secondes) ; `0` désactive le timer. |
| `encoded_subscription` | `true` | `/subscription` renvoie du base64 (compatible v2rayN / v2rayNG). |
| `prioritize_stability` | `true` | Re-vérifie le Top-N sauvegardé de la session précédente et les garde en tête, même si de nouvelles configs avec latence plus basse apparaissent. Avec `false`, préfère toute config fonctionnelle à faible latence. |
| `return_configs_asap` | `false` | Avec `true`, publie les configs fonctionnelles dès leur découverte (max `top_n`) ; les premières configs peuvent ne pas avoir la meilleure latence ou stabilité. |
| `scan_all_configs` | `false` | Avec `true`, vérifie toutes les configs chargées au lieu de s'arrêter après un nombre suffisant. |
| `fetch_timeout_ms` | `30000` | Délai de récupération par source. |
| `fetch_concurrency` | `8` | Nombre de sources récupérées en parallèle. |
| `max_subscription_bytes` | `33554432` | Taille maximale par source (32 Mio). |
| `use_cache_only` | `false` | Sauter la récupération en ligne et charger les configs précédemment testées depuis la base — utile sur les réseaux très restreints. |
| `emergency_config` | `null` | Lien de partage optionnel utilisé comme passerelle via `sing-box` lorsque la récupération HTTP échoue. |
| `clean_offlines_after_days` | `7` | Nombre de jours après lesquels les configs indisponibles sont supprimées de la base. |
| `sharing.enabled` | `false` | Autorise les clients LAN à accéder aux endpoints. |
| `sharing.require_token` | `false` | Les requêtes LAN nécessitent `?token=...`. |
| `sharing.token` | `null` | Vide = désactivé, `true` = génération automatique, chaîne = valeur exacte. |
| `proxy.enabled` | `false` | Démarre un processus SOCKS5/HTTP persistant via `sing-box`. |
| `proxy.port` | `27910` | Port du proxy mixte SOCKS5/HTTP. |
| `proxy.discoverable` | `false` | Lie sur `0.0.0.0` et ajoute une règle de pare-feu pour l'accès LAN. |
| `proxy.health_check_url` | `https://www.gstatic.com/generate_204` | URL testée via le proxy pour vérifier son état. |
| `proxy.health_check_interval_seconds` | `60` | Secondes entre les vérifications de santé. Bascul automatique en cas d'échec. |
| `probe.mode` | `active` | `active` utilise `sing-box` ; `tcp` est uniquement diagnostique. |
| `probe.sing_box_path` | `null` | Chemin optionnel vers `sing-box`. Laissez `null` pour les builds `_with_singbox` ou le chemin du package Termux. |
| `probe.connect_timeout_ms` | `5000` | Délai de connexion TCP en mode diagnostique. |
| `probe.active_timeout_ms` | `30000` | Délai du test HTTP en mode actif. |
| `probe.startup_timeout_ms` | `5000` | Temps d'attente du démarrage du proxy temporaire. |
| `probe.concurrency` | `16` | Nombre de base de vérifications actives simultanées. |
| `probe.batch_size` | `20` | Taille initiale du lot de vérification active. |
| `probe.process_concurrency` | `null` | Nombre de processus `sing-box` simultanés ; auto-ajusté si vide. |
| `probe.test_url` | `https://www.gstatic.com/generate_204` | URL de test chargée via chaque candidat. |
| `probe.accepted_statuses` | `[204, 200]` | Codes HTTP considérés comme succès. |
| `probe.download_url` | `null` | Cible optionnelle de test de débit. |
| `probe.download_bytes_limit` | `1048576` | Nombre maximal d'octets lus par test de vitesse. |
| `geoip_db_path` | `null` | Chemin optionnel vers un fichier `GeoLite2-Country.mmdb`. Si `null`, utilise la base intégrée pour la détection de pays. |
| `subscriptions` | _(sources pré-sélectionnées)_ | Liste de sources `{ name, url, enabled, priority }`. Ajoutez les vôtres pour une meilleure couverture. |

</details>

## 🌐 Notes pour les réseaux restreints

- Sur les réseaux très restreints, les configs précédemment testées sont stockées en base et accessibles via `use_cache_only: true`.
- Par défaut, si certaines URLs HTTP échouent mais qu'une config fonctionnelle est disponible, l'application l'utilise pour réessayer les abonnements échoués. Si aucune config n'est disponible mais que vous en avez une, ajoutez-la dans `emergency_config` du `configs.yaml`.

## 📡 Connecter vos clients à V2RayDAR

- **v2rayN (même PC)** — gardez `bind: 127.0.0.1:27141` et ajoutez `http://127.0.0.1:27141/subscription` comme URL d'abonnement.
- **v2rayNG / téléphone sur le même Wi-Fi** — liez-vous à l'IP LAN du PC (ex. `192.168.1.23:27141`), activez `sharing.enabled`, puis utilisez `http://192.168.1.23:27141/subscription` sur le téléphone. Vérifiez d'abord `/health` depuis le téléphone.

Le guide complet de configuration des clients, le partage protégé par token et les détails de pare-feu par OS sont dans le [guide développeur](guide.md).

### 📱 Proxy persistant pour le trafic des applications

V2RayDAR peut exécuter un proxy SOCKS5/HTTP persistant à côté de l'endpoint d'abonnement. Toute application sur le système — Telegram, navigateurs, curl, Python — peut y router son trafic sans client VPN séparé.

**Activer dans `configs.yaml` :**
```yaml
proxy:
  enabled: true
  port: 27910
  discoverable: false   # true = accès LAN + règle de pare-feu
```

**Usage local (sur l'appareil exécutant V2RayDAR) :**
```bash
curl --socks5 127.0.0.1:27910 https://api.ipify.org
```

**Usage LAN (téléphone sur le même Wi-Fi) :**
1. Réglez `proxy.discoverable: true` — V2RayDAR ajoutera une règle de pare-feu et écoutera sur `0.0.0.0`.
2. Trouvez l'IP LAN de l'appareil exécutant V2RayDAR dans le panneau TUI sous **Network** (ou exécutez `ipconfig` / `ip addr`). Par exemple `192.168.1.2`.
3. **Telegram :** remplacez `YOUR_LAN_IP` par votre vraie IP LAN et ouvrez cette URL sur le téléphone :

   ```
   https://t.me/socks?server=YOUR_LAN_IP&port=27910
   ```

   Par exemple, si votre IP LAN est `192.168.1.2` :
   ```
   https://t.me/socks?server=192.168.1.2&port=27910
   ```

   Ou manuellement : Telegram → Paramètres → Données et stockage → Paramètres du proxy → Ajouter un proxy :
   - Type : **SOCKS5** ou **HTTP**
   - Host : `YOUR_LAN_IP` (l'IP affichée dans le panneau TUI de V2RayDAR)
   - Port : `27910`

4. **Globalement sur Android :** Paramètres → WiFi → appui long sur le réseau → Modifier → Avancé → Proxy → Manuel → Serveur : `YOUR_LAN_IP`, Port : `27910`.

## 🤝 Contribuer

Les contributions sont les bienvenues ! N'hésitez pas à ouvrir un Issue pour les bugs, demandes de fonctionnalités, questions ou suggestions, ou à soumettre un Pull Request.

## 🗺 Feuille de route

- [ ] Ajouter une application GUI multiplateforme à côté du TUI avec Tauri.
- [ ] Extraire les configs V2Ray du corps de n'importe quel site web — de préférence les sites peu chargés en JS, avec FireCrawl ou Obscura en alternative pour les sites JS-intensifs.
- [ ] Endpoints privés avec mot de passe et authentification : quand un endpoint d'abonnement est privé et protégé par mot de passe, les utilisateurs peuvent obtenir leur endpoint privé via un endpoint national accessible.

## 👨‍💻 Avertissement

L'application est fournie « en l'état », sans aucune garantie.

Le développeur ne crée ni ne distribue de configs compatibles V2Ray, et n'est pas responsable des abonnements V2Ray que l'utilisateur scanne et utilise.

## ☕️ Contact et dons

### 💬 Contact

<p align="center">
<a href="https://t.me/TechKrakenBot">
  <img src="https://img.shields.io/badge/Telegram-2CA5E0?style=for-the-badge&logo=telegram&logoColor=white" alt="Telegram Bot">
</a>
</p>

### 💎 Dons via TON

Si vous trouvez ce projet utile, vous pouvez soutenir son développement par des dons sur la blockchain TON :

```
ton://transfer/TechKraken.ton
```

```
UQCGk4IU5nm6dYWjXTx6vSQVOtKO4LQg3m8cRcq1eQo7vhCl
```
