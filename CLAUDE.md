# Isola di Roccavento — guida per i coding agent

## Fonti di verità

Leggere all'inizio di ogni sessione:

0. **`docs/00-SPECIFICHE.md`**: le specifiche del nuovo progetto, nel testo dell'utente, con le risposte del 2026-10-14.
1. **`docs/01-GIOCO.md`**: le regole in vigore (che cosa deve fare il gioco).
2. **`docs/02-TERRITORIO.md`**: l'isola, gli incendi, come si modifica il territorio.
3. **`docs/03-VERIFICHE.md`**: le **fasi** con i loro checkpoint e la batteria di prove.
4. **`docs/TODO.md`**: la fase corrente, il lavoro aperto e le decisioni recenti.

I documenti dicono **cosa** costruire; il codice mostra **come**. Se non coincidono, prevalgono i documenti: segnalare il conflitto. Ogni nuova regola o decisione dell'utente va scritta, con la data, nel documento giusto. Lo storico non si conserva nei documenti: sta nella storia git. Documenti di servizio: `docs/MARIO.md` (easter egg), `docs/web.md` (versione browser), `docs/factory/<tema>/` (prove sul territorio). Metodi generici per serious game di protezione civile, riusabili in altri progetti: le skill in `.claude/skills/` (2026-10-20).

## Il gioco in breve

Nuovo progetto dal 2026-10-14. È uno strategico in tempo reale per chiunque apra la pagina web o scarichi la versione desktop (dal 2026-10-19 non è più un chiosco presidiato): italiano, mouse, trackpad o touch, nessun audio necessario. Un incendio simulato minaccia i borghi di un'isola.

- **Tutorial** scriptato e guidato, poi la **partita**: un incendio da affrontare.
- Il tempo scorre a ×300. La **pausa** (il mondo si ferma, senza limite di tempo né attesa per la successiva) è la richiesta della previsione all'esperto, ed è l'unico momento per dare ordini: trascinare le icone (badge) delle squadre (autobotte con 3 persone) sugli esagoni di 500 m ed evacuare un borgo alla volta. La partita parte in pausa.
- Alla fine: stelle e classifica del giorno.

Dettagli in `docs/01-GIOCO.md`.

## Architettura

**Una sola autorità di simulazione e gameplay: `rocca::Game`**, condivisa da runner headless, test e gioco. Nella grafica non vanno fisica, punteggio o logica.

| Crate | Contenuto |
|---|---|
| `propagator-core` | Il nucleo PROPAGATOR (automa cellulare del fuoco, checkpoint). Non sta in `crates/`: è una dipendenza Git da `CIMAFoundation/propagator_sim` (`rust/propagator-core`), fissata a un commit di `dev` nel `Cargo.toml` della radice (2026-11-05). Una modifica al nucleo si fa là, poi si sposta `rev`. |
| `fire` | `FireSim`, esposizione delle case (`exposure`), pericolo per chi è all'aperto (`threat`), ensemble per la previsione (`ensemble`). |
| `scenario` | Lettura dei dati dello scenario; `scenario::id()` legge `ROCCA_SCENARIO`. |
| `abm` | Rete stradale (`network`), guida (`drive`), squadre e spegnimento (`suppression`), aree sicure (`refuge`). |
| `behavior` | Grafi di comportamento delle squadre (`data/behaviours`). |
| `rocca` | Il gioco: `game` (pausa, ordini, passo), `hex`, `evac`, `tutorial`, `forecast`, `score`, `district`, `case`, `i18n` (lingue: `tr!`, cataloghi in `data/i18n/`). CLI `rocca`, esempio `tutorial_cerca`, test. |
| `game` | Il gioco Bevy (la cartella e le variabili si chiamano ancora `kiosk`/`KIOSK_*`): `sim.rs` (orologio attorno a `Game`), `kiosk/` (flusso, `ui`, `marks` sulla mappa, camera e clic in `view`, `snap`), rendering del diorama, `forecast_view`. Gamepad come mouse virtuale (`pad.rs`, spento mentre Mario è attivo). Mario (`mario.rs`, Ctrl+Maiusc+M a mondo fermo). |
| `datafs`, `libsm64-native` | Accesso ai dati (anche dal web); Mario. |

Dati: `data/scenarios/rocca_ventosa/`, con territorio e `game.json`: stazioni, squadre, regole dell'evacuazione, tutorial, incendi (`tutorial` e la partita `medium`). Python per il territorio: script autonomi in `tools/factory/`.

## Come si lavora

- **Fasi, una alla volta** (`docs/03-VERIFICHE.md`). Ogni fase si chiude con un checkpoint umano: la fase successiva non parte senza conferma. Le richieste puntuali dell'utente si fanno subito, ma non fanno avanzare la fase da sole.
- Prima di cambiare **regole, bilanciamento o territorio**: prova headless e tabella, poi decide l'utente. Una meccanica nuova va accesa nel gioco solo dopo la sua conferma, e va detto esplicitamente quando resta spenta.
- Il bilanciamento si giudica con un criterio: **nessuna strategia semplice deve dominare** («tutte le squadre sull'innesco subito», «evacua tutto»). Le prove usano gli stessi semi (1–3).
- **Resoconti all'utente in italiano**, al massimo una pagina: realizzato, evidenza (comando e tabella o immagine), non risolto, una proposta.
- **`docs/TODO.md`** va aggiornata nello **stesso commit** del lavoro: fase corrente e prossimo checkpoint in cima, aperti, decisioni con data. Le voci chiuse si cancellano: la storia è in git.
- **Conservare ciò che serve, adattare ciò che può servire, scartare il resto.** Nessuna riscrittura totale senza una ragione verificata. Le API si cambiano dopo averne controllato gli utilizzatori.

### Altre sessioni nello stesso repository

Spesso più sessioni Claude lavorano in parallelo sullo stesso checkout (oggi `main`; `nuovo-gioco` è stato unito e cancellato il 2026-11-02).

- Prima di mettere file in stage, guardare `git diff`: **non committare modifiche altrui**. Se un file è misto, chiedere alla sessione che lo sta modificando, o all'utente.
- Coordinarsi con `ListAgents` e `SendMessage`: chi tocca cosa, chi committa per primo.
- I sotto-agenti con `isolation: worktree` partono da `main`, non dal branch di lavoro. Farli lavorare nel checkout principale, su file distinti, senza commit, e fare il commit alla fine.
- Prima di un commit su file misti, verificare che l'albero in stage compili.
- Una copia dell'albero (`git archive`) che compila nella stessa `target/` condivide con il checkout le impronte di cargo, perché i percorsi sono relativi alla radice del workspace: dopo una build della copia, `cargo build` nel checkout può dire `Finished` senza compilare e lasciare in `target/release/game` il binario della copia. Dare alla copia un `--target-dir` suo, o toccare un sorgente del checkout prima di compilare (2026-11-06).

## Comandi

```
cargo build --release -p rocca --examples          # prove headless (docs/03-VERIFICHE.md)
cargo test --release -p rocca
TRACCIA=1 target/release/rocca tutorial
target/release/rocca medium --squadra "20:0=5590;4400" --evacua "20:Le Terrazze"
cargo run -p game --release                        # il gioco (desktop)
scripts/build_web.sh                               # versione browser, WebGL2 e WebGPU (docs/web.md)
```

Variabili utili del gioco (prefisso storico `KIOSK_`):

- `KIOSK_WINDOWED=1 KIOSK_SIZE=1600x1000 KIOSK_SCALE=1`: finestra e dimensioni.
- `KIOSK_QUALITY=alta|media|bassa`, `KIOSK_GFX`: livello grafico (`KIOSK_GFX=scale=0.5`: la scena 3D a metà risoluzione per lato; sul web `?gfx=scale=0.5`).
- `KIOSK_ADAPT=1`: la grafica adattiva, risoluzione compresa, anche con `KIOSK_SNAP`, senza toccare il gradino salvato: per vederla lavorare con `KIOSK_SNAP_BENCH` (righe `grafica adattiva` nel log).
- `KIOSK_FPS=1`: misura degli FPS.
- `KIOSK_VEG_STATS=1`: una riga al secondo con i milioni di triangoli delle piante nel passaggio principale e in ogni cascata d'ombra, per fascia di distanza (`vegetation/render.rs`). `KIOSK_SCANS=1`: lo stile realistico, in prova: piante scansionate, terreno, erba vicina, edifici e rocce con texture fotografiche (`assets/ground/`, `assets/buildings/`; tutto dietro `models::scans().is_some()`); `KIOSK_SCAN_LOD=<vicino>,<medio>,<lontano>,<grumi>` ne sposta le soglie, `KIOSK_SCAN_FAR=grumi` ridà ai loro modelli lontani i grumi di prima dei voxel. `KIOSK_VOLUMI=1`: fiamme e fumo volumetrici (in prova). Sul web, nella build WebGPU: `?scans=1`, `?volumi=1` (`docs/web.md`).
- `KIOSK_MEMORY=risparmio|normale`: forza o spegne la scena leggera per macchine con poca memoria (`memory.rs`; da sola scatta con RAM ≤ 8 GiB; dal 2026-11-02 un crash non la accende più).
- `KIOSK_PIANTE=1`: al posto del gioco, il laboratorio del fuoco sulle piante (`veg_lab.rs`): ogni varietà in fila, colonne che prendono fuoco a intervalli, cursore del tempo. `KIOSK_PIANTE_SNAP=<png>` salva una foto senza finestra (con `KIOSK_PIANTE_ROW`, `KIOSK_PIANTE_T`, `KIOSK_PIANTE_STEP`).
- `KIOSK_SCORES=<file>`: classifica in un altro file, per le prove (non toccare `~/.rocca_ventosa/`).
- `ROCCA_SCENARIO=<id>`: un altro territorio.
- `ROCCA_LANG=<codice>`: la lingua (`de`, `en`, …, `tlh` è il klingon); sul web `?lang=`. Ogni testo nuovo va in `tr!("…")` e `python3 tools/i18n/keys.py --check` deve passare (`docs/01-GIOCO.md` §7).

Il seme non si sceglie più dal gioco: F2 è il menu delle impostazioni (lingua, vista, grafica; 2026-10-21); il seme si dà alle prove headless.

### Immagini comparative degli scenari (senza finestre)

Lo script **`tools/factory/scenario_images.py`** salva le tre mappe comparative
mostrate in chat: stessa isola, scala e seme 2, senza ordini. Usa Matplotlib con
backend `Agg` (dipendenze Python: NumPy e Matplotlib); l’esempio Rust
**`crates/rocca/examples/scenario_maps.rs`** esporta gli arrivi reali del fuoco,
le case e gli impatti per borgo da `rocca::Game`.

Dalla radice del progetto, con gli esiti della suite già disponibili
(`variety-cases.json` e `variety-candidates.json` in
`results/scenario-impacts/focused/`):

```sh
cargo run --offline --release -p rocca --example scenario_maps -j 1 -- results/scenario-impacts/focused/variety-cases.json results/scenario-impacts/focused/images/data 2
MPLCONFIGDIR=/tmp/rocca-mpl .venv/bin/python tools/factory/scenario_images.py
```

Output: `results/scenario-impacts/focused/images/01_valico.png`,
`02_rotazione_est.png` e `03_quattro_borghi.png`. Per ricreare gli input della
ricerca, vedere [`tools/factory/SCENARI.md`](tools/factory/SCENARI.md).
**Non rubare il focus all'utente:** per queste immagini usare solo la procedura headless qui sopra. La finestra del gioco (`KIOSK_SNAP`, `cargo run -p game`) si avvia solo se l'utente chiede screenshot in quel messaggio.
**Audio sempre spento nelle prove** (utente, 2026-11-02): dalle casse non deve uscire niente. `KIOSK_SNAP` mette a 0 il volume globale di Bevy, che moltiplica ogni suono, Mario compreso; `rec=` registra la traccia senza suonarla; il laboratorio delle piante non ha suoni. Nei browser headless Playwright zittisce solo Chromium: per WebKit gli script iniettano `MUTE` (`scripts/perf_web.py`, `scripts/playtest_mobile.py`), da copiare in ogni script nuovo.

### Screenshot di stati arbitrari (`KIOSK_SNAP`)

Per controllare grafica e UX **non giocare una partita**: `kiosk/snap.rs` porta il gioco a uno stato descritto in una riga e salva il PNG. Serve la build `--release`. Con `KIOSK_SNAP` la finestra è **invisibile e senza focus** e l’audio è a volume 0 (`main.rs`): il PNG viene scritto senza che compaia nulla sullo schermo (verificato su macOS, 2026-10-20).

```
KIOSK_WINDOWED=1 KIOSK_SIZE=1600x1000 KIOSK_SCALE=1 KIOSK_SNAP=<cartella> KIOSK_SNAP_SPEC='name=a,phase=intro;name=b,phase=tutorial,step=4,pause=1,team=0;name=c,t=40,pause=1,move=0:5590:4400,select=Le Terrazze,at=fire' cargo run -p game --release
```

- Gli stati sono separati da `;`, i campi da `,`. Chiavi:
  - `name`;
  - `phase`: `intro`, `tutorial`, `gioco` (predefinita), `fine`;
  - `step`: il passo del tutorial;
  - `t`: minuti simulati;
  - `pause=1`;
  - `move=<squadra>:<x>:<y>`, più ordini uniti da `+`;
  - `evac=<borgo>`;
  - `team=<n>`, con `hex=<x>:<y>` per l'anteprima dello spostamento;
  - `select=<borgo>`: l'anteprima dell'evacuazione;
  - `at`: `home`, `fire`, `district:<nome>`, `team:<n>` oppure `x:y`;
  - `dist`, `yaw`, `pitch`: l'inquadratura;
  - `ui=0`, `marks=0`, `iso=0`: spengono interfaccia, segni sulla mappa e isocrone. **Per gli snapshot di un elemento della scena vanno sempre messi.**
- Il mondo resta fermo allo scatto. Il tempo va solo avanti: un `t` minore del precedente rifà la partita da capo.
- **Dopo un avvio con `KIOSK_SNAP` guardare sempre l'immagine** (Read).
- Chiave `live=1`: il mondo continua a scorrere (per misurare anche i costi CPU del fuoco).
- **Mario** (`docs/MARIO.md`, serve la ROM): `mario=1` accanto al fuoco, `mario=<x>:<y>` in un punto dato, `walk=1` lo fa correre (con `rec`). Un solo stato con Mario per avvio. Esempio sul prato: `name=corsa,t=1,pause=1,mario=4730:6130,walk=1,ui=0,marks=0,iso=0,rec=4` con `KIOSK_REC_PNG=1`.
- Chiave `opening=<s>` (con `phase=title`): la sequenza d'apertura ferma a quell'istante; con `rec` la sequenza scorre.
- **Video** (`record.rs`, 2026-10-30): `rec=<s>` registra `<s>` secondi di quello stato in `<nome>.mp4`, immagini e suoni del gioco, a passo fisso di 1/30 s (fluido su qualsiasi macchina, più veloce del tempo reale). Serve ffmpeg. Con `KIOSK_SIZE=1920x1080 KIOSK_SCALE=1.25` l'immagine è uscita nera, anche negli snapshot (2026-10-30): per i video usare `KIOSK_SCALE=1`. Esempio: `name=apertura,phase=title,opening=0,live=1,rec=14`. Il montaggio del video di presentazione (voce TTS, cartello) sta fuori dal repository, in `~/Movies/rocca-demo/cut.py`.
- **Camera in movimento** (2026-11-02): `spin=<gradi/s>` (rotazione), `zoom=<m/s>` (negativo: avvicina), `pan=<x>:<y>` (m/s verso est e nord) muovono la camera dalla fine dell'assestamento, sul clock del gioco; con `rec` filmano lo stesso percorso su qualsiasi macchina. Con `KIOSK_REC_PNG=1` ogni fotogramma del video è salvato anche in PNG senza perdita (`<nome>_frames/f0000.png`): per confrontare fotogramma per fotogramma non usare l'mp4, i suoi B-frame sfocano un fotogramma su due. Esempio: `name=zoom,t=150,at=fire,dist=1600,pitch=-32,ui=0,marks=0,iso=0,zoom=-400,rec=3`.
- **FPS per stato:** `KIOSK_SNAP_BENCH=<s>` misura ogni stato per `<s>` secondi e stampa righe `bench:` (main, render thread, GPU, fps). La finestra nascosta è cadenzata a ~57 Hz e la GPU abbassa il clock, quindi alla dimensione reale i ms GPU non sono confrontabili: per un prima/dopo si usa `KIOSK_SIZE` a 4–8 volte i pixel (es. `3919x2449` con `KIOSK_SCALE=2`) finché il `mean` supera i 17,5 ms; lì è il costo vero. Dettagli in `crates/game/src/bench.rs`.

## Trappole note

Sono costate tempo e non si vedono dal codice. Prima di fidarsi, verificare che valgano ancora.

- **Le case non bruciano nell'automa:** stanno su celle non combustibili, come strade e persone. L'esposizione delle case è uno strato a parte (`fire::exposure`). Per chi sta all'aperto c'è un altro strato, `fire::threat`; i due non sono intercambiabili.
- **Un innesco di una sola cella** non attecchisce in circa il 20 % dei casi: usare sempre `FireSim::ignite_patch`.
- **Tutto ciò che si accumula va integrato sul tempo simulato**, non sul numero di update. Un controllo di invarianza al passo deve scendere anche sotto il ritmo interno del modello.
- **`wind_from_deg` / `w_dir` indica da dove soffia il vento** (il sorgente del kernel trae in inganno).
- **`vegetation_changes` è sparso tramite NaN:** una griglia piena di zeri riclassifica tutta la mappa.
- **Le strade dipinte come non combustibili:** il pericolo sulle strade viene da `fire::threat` vicino al fronte, non dalle celle stradali. Le strade chiuse si leggono da lì (`Game::closed_roads`).
- **Una mesh senza attributi** (vuota) manda in panico la pipeline di prepass di Bevy all'avvio: non lasciare mesh segnaposto vuote.
- **Una mesh solo sulla GPU** (`RenderAssetUsages::RENDER_WORLD`) resta in `Assets<Mesh>` dopo l'estrazione, ma senza vertici: `attribute()` e `indices()` vanno in panico (con Bevy 0.15 spariva dagli asset). Ciò che il gioco rilegge, come le collisioni di Mario (squadre, monumenti), tiene `MAIN_WORLD`; per leggere una mesh qualunque, le varianti `try_` (2026-11-02).
- **Il nodo carrabile più vicino** spesso è un'isola della rete: usare `nearest_reachable`. Mai un A* a ogni sotto-passo: si ricalcola quando cambia l'obiettivo o allo scadere di `REROUTE_S`.
- **Rendering:**
  - una striscia stesa sul terreno con il winding sbagliato è invisibile (avvolgerla `a, a+2, a+1`, e controllare in che verso corrono le righe del reticolo);
  - le linee drappeggiate vanno ricampionate, circa ogni 5 m;
  - i segni vanno alzati sopra la chioma (+20 m o più);
  - un `Material` personalizzato non riceve la nebbia;
  - il depth prepass non deve coprire più del main pass: dove il prepass scrive una profondità e il main pass lascia il campione scoperto (alpha to coverage, `discard`), dietro non si disegna più nulla e resta il colore di sfondo, grigio, che sfarfalla muovendo la camera (2026-11-02). Per questo le mesh ritagliate delle piante restano fuori dal prepass;
  - con la scena disegnata più piccola (`render_scale` < 1, risoluzione adattiva, 2026-11-02) il main pass occupa solo l'angolo in alto a sinistra della vista e viene allargato dopo: ciò che legge la profondità a risoluzione dello schermo non coincide più. La profondità di campo si spegne sotto 1 e Mario riporta la scala a 1; uno shader nuovo che ricava coordinate di schermo da `view.viewport` deve usare `view.main_pass_viewport`;
  - **una texture con mip riscritta dalla CPU** (2026-11-05): Bevy 0.20, se l'immagine ha `COPY_DST` (c'è di default), la aggiorna in place e solo nel livello 0; i mip restano quelli del primo caricamento. Una texture con mip che cambia (la mappa del pennacchio, `fire_volumetric`) va creata senza `COPY_DST`, così ogni aggiornamento la ricrea intera;
  - **il web ha due build** (2026-11-02): WebGL2 per telefoni e browser senza WebGPU, WebGPU (feature `webgpu`) per i computer, con la grafica del desktop (`docs/web.md`, «WebGPU sui computer»). Un limite di WebGL2 va dietro `quality::WEBGL2`, non dietro `target_arch = "wasm32"`. Il WGSL del browser rifiuta cose che naga accetta (texture in un `let`, derivate dopo un ramo non uniforme): uno shader nuovo va provato nella build WebGPU (`perf_web.py shots`, `?gpu=1`);
  - **sul web la profondità ha poca precisione** (WebGL tiene l'intervallo di OpenGL, il reverse-Z di wgpu perde il suo vantaggio): due superfici a meno di un metro, lontane qualche km, fanno z-fighting. Il piano vicino della camera web segue la distanza (`camera::web_near_plane`); un controllo che proietta un punto vicino alla camera deve stare oltre i 50 m (2026-11-02);
  - Bevy toglie di colpo l'ombra del sole alla fine dell'ultima cascata: la sfumatura sta in `scene_light::shadow_reach` e la leggono terreno e strutture (`faded_shadow::apply_pbr_lighting_faded`), piante e `scene_shade` (2026-11-02). Un materiale nuovo illuminato con l'`apply_pbr_lighting` di Bevy riporta la linea.
- **Rumore letto in `posizione / scala` con una scala che varia** (2026-11-05): le coordinate del mondo sono migliaia di metri, quindi una scala che cambia dell'1 % sposta la lettura di periodi interi. Se la scala dipende dalla distanza dall'occhio o da un campo che varia nello spazio, ogni campione di un ray-march legge il rumore a caso, il jitter ridisegna tutto a ogni fotogramma e l'immagine vibra (le fiamme volumetriche, `fire_march.wesl`). La scala va tenuta ferma per ottave e sfumata tra due. Per trovare un difetto così: colori piatti per stadio al posto dell'immagine (`KIOSK_VOL_SET=solo=1,marca=2`), non misure sull'immagine finale.
- **Direzione di un raggio come differenza di due posizioni del mondo** (2026-11-06): uno shader a schermo che ricava il raggio da «punto del piano vicino, nel mondo, meno l'occhio» sottrae due punti distanti 10 cm e lontani chilometri dall'origine, dove un `f32` avanza di mezzo millimetro. La direzione sbaglia fino a un centesimo di radiante, cioè pixel interi, con un reticolo che cambia a ogni movimento della camera: fumo e fiamme volumetrici vibravano nel gioco e non nel laboratorio, che sta all'origine. La direzione si ricava dalla sola rotazione della camera (`fire_volumetric::rays_from_clip`); la matrice del fotogramma precedente si dà relativa all'occhio di questo, in doppia precisione (`prev_from_eye`). Due regole di metodo: un difetto che c'è nel gioco e non in un laboratorio si cerca nelle differenze tra i due, stadio per stadio (`scarto=1` disegna quanto lontano da sé la storia ritrova ogni pixel: nero a camera ferma); la stabilità di un effetto a schermo si misura anche con la camera in movimento (`spin`, `zoom`), non solo ferma.
- **La griglia del fuoco ha quote sue** (2026-11-06): `dem.f64` (`Scenario::dem`, 20 m) non è il terreno disegnato (`render_terrain.f32`, 5 m, `Terrain::height_at`). Lungo il crinale dell'isola sta decine di metri più in alto (fino a 150 m); altrove coincidono, e per questo non si nota. Ciò che si disegna poggia su `height_at`, mai su `dem`: fiamme e fumo volumetrici, che prendevano la quota da `dem`, stavano sospesi in aria. E per ciò che è alto pochi metri non basta una quota per cella del fuoco: tra i centri cella il terreno disegnato si scosta di 2,5 m rms (`fire_volumetric::GROUND_MAP` porta agli shader il terreno a 5 m).
- **Un effetto a schermo steso dopo la risoluzione dei campioni** (2026-11-06): con il multisampling un pixel sul bordo di una foglia contiene due superfici, la foglia e ciò che le sta dietro, e ognuna ha davanti un fumo diverso. Fumo e fiamme volumetrici, ricomposti sull'immagine già risolta con una sola profondità per pixel, lasciavano il cielo senza fumo su quei bordi: ogni foglia contro il fumo aveva un contorno chiaro (sembrava il contorno di Mario: non lo era, c'è anche senza). La ricomposizione ora disegna nei campioni, prima che siano risolti, e due volte: la superficie lontana di ogni pixel, poi (`NEAR`) la vicina dei pixel che ne hanno due, ciascuna con `sample_mask` sui suoi campioni (`fire_composite.wesl`, `surface`). Stessa radice per la storia a un quarto di risoluzione: un pixel di marcia sul bordo di una foglia che ondeggia è ora foglia ora cielo, e la media nel tempo va limitata ai vicini della stessa superficie (`fire_resolve.wesl`), altrimenti attorno alle foglie resta un alone morbido. Per riconoscere i due difetti: il contorno sparisce con `KIOSK_GFX=msaa=1`, l'alone con `KIOSK_VOL_SET=storia=0`. Vale per ogni effetto a schermo nuovo che legga la profondità.
- **Le ombre costano più di ciò che fa ombra** (2026-11-06): ogni cascata del sole ridisegna ciò che sta nel suo riquadro, e il riquadro è un quadrato attorno alla fetta di vista, non la fetta. Le piante vicine, al livello più fine, entravano in tutte e quattro le cascate: 88 milioni di triangoli d'ombra per 13 disegnati. Una cascata è letta solo da ciò che sta oltre il suo inizio (0,7 del limite della precedente): più vicino, un oggetto può gettare una mesh più grossolana o niente (`vegetation::render::pick_caster`, `slice_outline`). Prima di ottimizzare un passaggio si contano i triangoli per passaggio (`KIOSK_VEG_STATS=1`), e le misure di GPU si fanno senza altri processi grafici, due volte e alternate: quelle prese mentre girava un altro gioco erano tre volte più alte.
- **Il tempo GPU di un passaggio** (2026-11-05): sulle GPU Apple il timestamp d'inizio di un pass è l'inizio del suo stadio dei vertici, che gira mentre i pass precedenti disegnano ancora: misurava dentro anche il main pass. Si usano le fini dei pass e le loro differenze (`bench::PassStamps`). E un confronto tra due build di qualcosa che si muove sul clock degli shader va fatto a passo fisso (`TimeUpdateStrategy::ManualDuration`, come il laboratorio del fuoco): così due avvii danno la stessa immagine pixel per pixel (verificato), mentre sul clock reale ogni avvio fotografa un istante diverso.
- **Riavvio:** vanno ripuliti gli stati del rendering che si ricordano qualcosa (case annerite, particelle di fumo, chiavi dei segni). Tutto il resto si ricalcola da `Sim` a ogni frame.
- **Scorciatoie da tastiera:** devono rispettare `egui` `wants_keyboard_input()`, altrimenti scattano mentre si scrive il nome.
- **Un meccanismo nuovo deve essere inerte quando è spento**, anche a T+0. Una derivazione giusta su un territorio può essere sbagliata sul successivo: va verificata su tutti.
- **Ciò che il giocatore vede e ciò che il finale conta** devono venire dalla stessa regola.
