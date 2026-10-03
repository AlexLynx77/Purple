> **Journal historique de mise au point.** Ce fichier garde la trace de la recherche et des diagnostics, dans l'ordre. Certains passages sont obsolètes (les « spikes » ont été supprimés, les chemins `C:\tmp\lol` n'existent plus, `mod-tools.exe` n'est plus utilisé). L'état à jour du projet est dans le [README](../README.md).

# Purple — notes de recherche

Objectif : un skin changer LoL perso, sans dépendre d'une app tierce qui peut s'arrêter.
Références étudiées : Rose (Alban1911/Rose), LTK Manager + league-mod (LeagueToolkit).

## Comment ça marche (3 couches)

1. **Fichiers de skin** — un skin = un petit `.fantome` (zip) contenant `WAD/<Champion>.wad.client`.
   Ex. `LeagueSkins/skins/1/1001/1001.fantome` (Goth Annie) = 5 Ko : un `skin0.bin` (format PROP)
   qui *redirige* vers `DATA/Characters/Annie/Skins/Skin1.bin`, + un bin pour les sous-personnages
   (Tibbers). Les vrais assets (modèles, textures) sont déjà dans l'install du jeu.
   → on peut les **générer nous-mêmes** plus tard ; en attendant on utilise LeagueSkins.
2. **Overlay** — copie des WAD du jeu avec les chunks du mod. Construit par `mod-tools.exe mkoverlay`
   (cslol, spike) ou par la crate Rust `ltk_overlay` (cible finale). ~2 s pour un skin.
3. **LTK Patcher** — `ltk_patcher_host.exe` + `ltk_patcher_dll.dll` (binaires fermés, maintenus par
   LeagueToolkit, licence permissive : réutilisable/redistribuable, re-signer si redistribué hors
   release officielle). Le host injecte la DLL dans le jeu au lancement ; la DLL fait lire au jeu
   l'overlay à la place des WAD officiels (et contourne la vérification de signature).

## Protocole du host (stdin/stdout, texte)

```
config loglevel 16
config flags 4            # 4 = OPT_OUT_AH_V1 : désactive le scan anti-skinhack (obligatoire pour nous)
config prefix <overlay>\  # dossier overlay, avec antislash final
start scan                # puis lancer le jeu APRÈS (la DLL ne patche que les jeux lancés après)
stop
```
Events : `status <ts> injecting|injected|waiting|exited|failed ...`, `error ...`, `dll <ts> <pid> <tid> <level> <msg>`.
Vérifié en local : le host répond `ok` aux 3 config puis `status ... injecting scanning for game`.

## Risques / dépendances

- La DLL a une **date de fin de vie** (`end of life reached, please update`) : elle refuse les builds
  du jeu trop récents → il faut récupérer les nouveaux binaires LTK à chaque patch. Prévoir une
  mise à jour auto depuis les releases de LTK Manager + patcher derrière une interface remplaçable.
- Le scan anti-skinhack de la DLL bloque les skins portés sur le champion de base (`c0000229`)
  → flag 4 (comme Rose).
- Rose suspend le processus du jeu pendant la construction de l'overlay ; non nécessaire si l'overlay
  est prêt avant le lancement du jeu.
- Télémétrie possible dans les binaires LTK : non vérifié.

## Environnement de dev (cette machine)

- WSL ne démarre pas (WSL1 non supporté) — non nécessaire, Rust est installé sous Windows.
- Rust MSVC installé mais **pas de Build Tools C++** (VS Code seulement) → pas de linker.
  Solution retenue (2026-10-03) : **w64devkit 2.10.0** (gcc 16.2) dans `C:\Users\pierr\tools\w64devkit`
  + toolchain Rust `stable-x86_64-pc-windows-gnu`, épinglée par `rust-toolchain.toml`, chemins dans
  `.cargo/config.toml`. Rien dans le PATH système. Deux rustines : un `lib/libgcc_eh.a` vide ajouté dans
  w64devkit (Rust le réclame, w64devkit n'en a pas) et `-C dlltool=` dans `.cargo/config.toml`.
  Testé : la crate `zstd` (code C) compile. Si le dossier w64devkit est supprimé : refaire les rustines.
- Le stub `cslol-dll.dll` du spike a été lié à la main avec `rust-lld` (no_std, aucun import).

## Choix techniques

- Langage : **Rust** (crates `ltk_*` natives, binaire minuscule, RAM minimale).
- UI : native légère (egui ou Slint) plutôt que Tauri/WebView2 pour la RAM.
- Pipeline cible : LCU websocket (champ select) → UI choix de skin → `ltk_overlay` → host patcher.

## Spikes (`spikes/`)

- `setup.sh` : copie `mod-tools.exe`, `ltk_patcher_host.exe`, `ltk_patcher_dll.dll` dans `spikes/tools/`
  (sources : clones locaux de Rose et ltk-manager dans `C:\tmp\lol`). Gitignorés.
- `spike1.py <fichier.fantome>` : unpack → mkoverlay → host → attend le lancement d'un jeu.
- `cslol_stub/` : DLL vide `cslol-dll.dll` requise par `mod-tools.exe` pour démarrer.

## Résultat spike 1 (2026-10-03) — échec partiel, instructif

- Injection OK : host → `game found` → `dll attached` → `overlay verified 1 wad(s)` → `redirected wad: Annie.wad.client`.
- Puis le jeu meurt ~1 s après : `FATAL ERROR - Installation is corrupt. WadFile mount failed.
  DATA/FINAL/Champions/Annie.wad.client — Problem: Corrupt`, et il écrit `SOFT_REPAIR` (le client
  lance alors sa réparation officielle, d'où la popup « problème avec votre installation »).
  Les fichiers du jeu ne sont pas touchés (l'overlay est dans un dossier séparé).
- Cause probable (NON confirmée) : `mod-tools.exe mkoverlay` (cslol, ancien) réécrit le WAD sans
  gérer correctement les chunks WAD v3.4 « zstd + sous-chunks » (type 4) : les index de sous-chunks
  de l'overlay diffèrent de l'original pour 1562 chunks sur 4083. Le jeu actuel rejette le résultat.
- À tester : construire l'overlay avec la crate moderne `ltk_overlay` (ce que fait LTK Manager)
  → nécessite un compilateur (MSVC Build Tools ou MinGW) ; ou tester LTK Manager en oracle.
- Le scan anti-skinhack échoue bien comme prévu (`c0000229`, « opted out » grâce au flag 4) : OK.

## Résultat spike 2 (2026-10-03) — SUCCÈS

- `spikes/overlay` (Rust, crate `ltk_overlay` 0.16.2 + `FantomeContent`) construit l'overlay en ~0,6 s.
  Seuls 2 chunks diffèrent de l'original (contre 1562 avec mod-tools.exe) : la cause du crash du spike 1
  était bien le réécriture du WAD par l'ancien cslol.
- Test en jeu : Goth Annie affichée sur le skin de base (Outil d'entraînement). Pipeline validé :
  `.fantome` LeagueSkins → `ltk_overlay` → `ltk_patcher_host` (flags=4) → jeu.
- `mod-tools.exe` et le stub `cslol-dll.dll` ne sont plus nécessaires (spike 1 obsolète).
- `spikes/host.py <overlay_dir>` lance seulement le host sur un overlay déjà construit.

## Prochaines étapes
1. Binaire `purple` (Rust) : LCU (lockfile + websocket) → détecte champ select / champion / skin voulu.
2. Télécharger/mettre en cache les `.fantome` LeagueSkins (index par champion/skin, chromas).
3. Piloter le host depuis Rust (spawn, protocole texte, events), démarrer le scan avant le lancement du jeu.
4. UI native légère (egui/Slint) + tray ; mise à jour auto du couple patcher host+DLL (date de fin de vie).
5. Plus tard : générer les skins localement au lieu de dépendre de LeagueSkins.

## Binaire `purple` (crates/purple) — état au 2026-10-03

Commandes : `status`, `skins <champion>`, `set <champion> <skin>`, `unset`, `list`, `run`, `build <champ id> <skin id>`.
`run` : poll du client (gameflow-phase + champ select), au champion choisi → télécharge le .fantome
(LeagueSkins, cache `%LOCALAPPDATA%\Purple\skins`) → `ltk_overlay` → host patcher (`start scan`) ; ne coupe
jamais le patcher tant qu'un jeu est attaché. Env : `PURPLE_DEBUG=1` (logs complets), `PURPLE_DATA_DIR`,
`PURPLE_PATCHER_DIR`. Binaires du patcher : `vendor/ltk-patcher/` (licence incluse).

### Bug résolu : « overlay verified 0 wad(s) » (aucun skin visible) — CONFIRMÉ en jeu
- Symptôme : injection OK, mais le DLL ne voit aucun WAD dans l'overlay → skin par défaut.
- Cause : l'overlay était sous `%LOCALAPPDATA%`, donc sous `C:\Users\<nom>`, dont l'ACL n'autorise que le
  compte, SYSTEM et les admins. Le jeu ne peut pas traverser ce dossier. Donner la lecture au seul dossier
  final (essai `icacls` sur `overlay\`) ne change RIEN : diagnostic incomplet, réfuté en jeu.
  Même overlay, octet pour octet, dans `C:\tmp\pd` ou `C:\ProgramData\Purple` → ça marche.
- Correctif : les overlays vivent dans `%PROGRAMDATA%\Purple` (lisible par le groupe Utilisateurs, hérité par
  les fichiers). Cache des skins et config restent dans `%LOCALAPPDATA%\Purple` (le jeu ne les lit pas).
- Validé en jeu le 2026-10-03 : Annie gothique (1001) et Aatrox primordien (266033), via `purple run`,
  chemins par défaut. Logs : `AH wad scan failed c0000229 ... (opted out)` → `overlay verified 1 wad(s)` →
  `redirected wad: .../<Champion>.wad.client`.
- `run` affiche une alerte explicite si le DLL répète `overlay verified 0 wad(s)`.
- Mesure : `purple` ~15 Mo de RAM + `ltk_patcher_host` ~8 Mo ; binaire release 3,3 Mo.

## Fenêtre (`purple.exe`) — 2026-10-03

- Structure : lib `purple` (catalog, config, engine, gui, images, lcu, overlay, patcher, store) + 2 binaires :
  `purple` (fenêtre, sous-système Windows en release, pas de console) et `purplectl` (ligne de commande).
- `engine::run` = boucle partagée (événements `engine::Event`) ; la fenêtre la lance dans un thread, `purplectl run`
  l'affiche dans le terminal. La fenêtre écrit `config.json`, le moteur le relit à chaque seconde de champ select
  (changer de skin pendant le champ select ré-arme).
- UI : egui/eframe 0.36 (backend `glow`, pas `wgpu` : plus léger). API 0.36 : `App::logic` + `App::ui`, `egui::Panel`.
- Images : portraits + tuiles de skins viennent du client (`/lol-game-data/assets/...`), cache disque
  `%LOCALAPPDATA%\Purple\img`, décodées/réduites dans un thread, textures plafonnées (240). Catalogue mis en cache
  (`catalog.json`) : la fenêtre s'ouvre même client fermé.
- Champions `Jade_*` (ids >= 60000, autre mode de jeu) masqués : ils doublonnaient tous les noms.
- La fenêtre refuse de se fermer pendant une partie (tuer le patcher sous le jeu = risque).
- Vérifié par captures + clics simulés : choix, annulation, bouton « skin par défaut », config.json mis à jour.
- Mémoire release : fenêtre ouverte ~115 Mo (surtout contexte OpenGL + polices + textures), contre ~15 Mo pour
  le moteur seul. Piste : démon tray (moteur seul, ~15 Mo) + fenêtre lancée à la demande dans un processus à part.
- Captures d'écran sous Windows : `SetProcessDPIAware` indispensable sinon la capture est rognée (échelle 125 %).

## Tray (2026-10-03) — architecture finale à 3 binaires

- `purple.exe` : programme résident (moteur + icône de zone de notification, tray-icon 0.26 + boucle de messages
  Win32). Release : ~15 Mo (3,6 Mo privés), aucune fenêtre. Menu : état (grisé), Ouvrir Purple, Quitter.
  Clic gauche sur l'icône = ouvre la fenêtre. « Quitter » pendant une partie = attend la fin de la partie.
  Instance unique (mutex `Local\Purple.Daemon`) ; un 2e lancement ouvre la fenêtre. Journal : `purple.log`.
- `purple-ui.exe` : la fenêtre (egui). Process à part, lancé à la demande, libère ses ~100 Mo à la fermeture.
  Instance unique (`Local\Purple.Window`, sinon focus sur la fenêtre existante, titre « Purple Skin Changer »).
  Lecture seule de l'état : `status.json` (écrit par le tray), écrit `config.json`. Affiche « Engine stopped » +
  bouton « Start it » si le tray ne tourne pas.
- `purplectl.exe` : ligne de commande (dev/diagnostic) ; ne pas lancer `purplectl run` en même temps que le tray.
- Le patcher est rattaché à un job Windows KILL_ON_JOB_CLOSE : il meurt avec `purple.exe` (crash, kill) au lieu de
  rester actif et de patcher une prochaine partie avec un vieil overlay.
- Le patcher est lancé avec CREATE_NO_WINDOW (sinon console noire visible quand le parent est sans console).
- Windows 11 range les nouvelles icônes dans le menu « ^ » (icônes cachées) : à glisser vers la barre.
- Reste à faire : lancement au démarrage de Windows, icône dans le .exe (ressource), noms de skins en anglais
  (CommunityDragon en_gb a les mêmes ids : 2159 skins ; actuellement noms dans la langue du client),
  mise à jour automatique du patcher LTK (date de fin de vie), chromas, skins déjà possédés.
