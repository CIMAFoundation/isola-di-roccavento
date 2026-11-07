# Versione per browser

La stessa partita della versione desktop, nel browser, per chi non la scarica. Stesso motore (`rocca::Game`) e stessi dati, scaricati a parte dal `.wasm` (vedi «Dati separati dal codice»; `crates/datafs`).

## Avvio su iPhone: crash segnalato (2026-10-21)

Segnalazione su iPhone 13, iOS 27, Chrome: chiusura quando compare titolo/mappa, ricarica automatica e poi «can't open this page». **Causa non confermata sul dispositivo**: il sintomo è compatibile con pressione di memoria, ma le precedenti misure erano solo della memoria lineare Wasm. Non rappresentano RAM totale, compilazione del codice o GPU.

Interventi locali:

- Bevy web con le sole funzionalità usate: esclusi renderer UI/testo di Bevy (l'interfaccia è egui), glTF, scene, animazioni e morph. Conservati audio/Mario, gamepad e rendering del gioco; desktop nativo con le feature precedenti. Il modulo passa da circa **61 a 48 MiB**;
- profilo `bassa`: texture grafica strade/costa a **8 m** anziché 4 m (un quarto dei texel), desktop web ancora a 4 m e nativo a 2 m. Dati della simulazione invariati;
- dati del telefono conservati gzip in JS, **4,54 MiB** anziché 15,89 MiB; due download dati simultanei (sei sul computer), dopo la compilazione Wasm sul telefono;
- sul telefono niente nuova copia Cache Storage del download Wasm durante la compilazione: si leggono cache già presenti, mentre sui miss si usa la cache HTTP. Evita la copia extra potenzialmente accumulata da [`Response.clone()`](https://developer.mozilla.org/en-US/docs/Web/API/Response/clone) quando i due lettori avanzano a velocità diverse;
- decompressione Rust con capacità iniziale ricavata dal footer gzip, per evitare riallocazioni dei grandi raster.

Verifiche: **25 test nativi superati**, maschera desktop identica, build wasm riuscita. WebKit emulato 390×844 a densità 3: titolo **256,1 MiB Wasm** contro 291,8 precedenti; tre ricariche consecutive, nome, tutorial e rotazione superati senza errori browser. Il tutorial sale a **292,8 MiB**. Chromium: `q=bassa` 256,25 MiB al titolo, **292,94 MiB** all'avvio della partita e fino al finale/riavvio; JS dati 4,54 MiB. I precedenti 291,88 MiB Wasm del profilo basso salivano meno entrando in partita, ma conservavano 15,89 MiB di dati JS. Questi numeri escludono codice compilato, resto del browser e GPU. Desktop web: media 357,63 MiB, adattiva 393,5 MiB; i tre flussi completi senza ordini passano. Screenshot verificati.

```sh
scripts/build_web.sh
.venv/bin/python scripts/playtest_mobile.py --browser webkit --smoke --reloads 3 --output out/playtest/iphone-reloads
.venv/bin/python scripts/playtest_memory.py --output out/playtest/iphone-memory-final
```

Report in `out/playtest/iphone-reloads/` e `out/playtest/iphone-memory-final/`. **Non provato su iPhone fisico e non pubblicato da questa sessione**: il confronto WebKit desktop non certifica il limite di memoria o la stabilità di iOS 27.

## Memoria condivisa con il desktop (2026-10-21)

Ottimizzazioni a parità di qualità grafica, senza cambiare la simulazione:

- modelli Blender deserializzati direttamente negli array tipizzati, senza albero `serde_json::Value` e copie intermedie;
- maschera di strade e costa: raster temporanei liberati prima del canale successivo, mipmap costruite in un unico buffer prenotato, ricerca del mare con coda e celle marcate all'inserimento;
- geometrie immutabili di costa, campi, modelli, vegetazione dettagliata e resti bruciati: copia CPU rilasciata dopo il caricamento GPU. Le proxy della vegetazione conservano la copia CPU perché il LOD cambia gli indici; le immagini degli stati restano modificabili;
- indici delle mesh statiche a 16 bit quando ci sono al massimo 65.535 vertici, senza perdita di precisione; le proxy modificabili mantengono gli indici a 32 bit;
- sul web le due entità della vegetazione condividono la stessa mesh proxy: prima la copia vicina, sempre nascosta, occupava comunque CPU e GPU.

Confronto Chromium headless, viewport 1280×800, contesti nuovi, stesso scenario `medium` senza ordini fino al finale, confronto senza ordini pronto e nuova partita. Capacità della memoria lineare Wasm, **non RAM totale del browser o GPU**; il valore rimane uguale dal titolo al riavvio in questa prova:

| Profilo | Prima | Dopo | Riduzione |
| --- | ---: | ---: | ---: |
| `q=bassa` | 342,3 MiB | 291,9 MiB | 14,7% |
| `q=media` | 458,4 MiB | 362,8 MiB | 20,8% |
| Adattiva desktop | 527,3 MiB | 402,4 MiB | 23,7% |

Sul desktop nativo, due coppie di esecuzioni alternate a **1280×800 effettivi**, qualità `alta`, scena a T+90 min (`at=fire,dist=2600`), finestra nascosta: massimo RSS medio **2607 → 2279 MiB** (−12,6%; prima 2471–2743 MiB, dopo 2278–2279 MiB). Misurato con `/usr/bin/time -l`. Il `peak memory footprint` di macOS varia molto: media 4045 → 4001 MiB; non è una misura stabile del risparmio GPU. Le prime prove fullscreen avevano risoluzioni diverse e sono escluse. Screenshot controllati; log e misure in `out/playtest/memory-desktop-comparison/` e `out/memory-desktop-{0,1}-{before,after}.log`. **25 test nativi passati**, incluso il fingerprint identico degli 85.332.856 byte della maschera desktop con tutte le mipmap.

Report JSON e PNG in `out/playtest/memory-game-before/` e `out/playtest/memory-final/`. Nessun errore browser nei tre flussi. Riproduzione:

```sh
scripts/build_web.sh
.venv/bin/python scripts/playtest_memory.py
.venv/bin/python scripts/playtest_mobile.py --smoke
```

Il test touch ha ora una lingua esplicita (`it-IT`), coerente con i testi che cerca. Dal 2026-11-03 la prova completa passa tutta (prima si interrompeva nel trascinamento del tutorial: il badge trascinava con sé la mappa, voce in `TODO.md`). La misura della partita completa usa il runner di memoria separato. Avvio, nome, tutorial e rotazione verificati con `--smoke`; nessuna prova su telefono fisico.

## Pagina introduttiva (2026-10-18)

`web/index.html` mostra prima una pagina HTML+CSS nello stile di cimafoundation.org (hero con una scena di incendio e un riquadro «Dal gioco», tre card, un paragrafo sulla ricerca, chiusura, crediti). Testi brevi e narrativi, senza meccaniche, durata o numeri (2026-10-19, richiesta dell'utente). `game.js` e il `.wasm` si scaricano solo quando si preme «Gioca» (`import()` dinamico in `start()`); con `?playtest=1` si salta l'intro. Dopo «Gioca» compare la schermata del titolo del gioco, con il suo «Gioca» (decisione dell'utente, 2026-10-18: non va saltata). Immagini in `web/img/`, copiate da `build_web.sh`: screenshot veri del gioco (rifatti il 2026-11-02 con la grafica nuova; `KIOSK_SNAP`, 1600×1000, `KIOSK_GFX=-dof KIOSK_TACTICAL=0 KIOSK_BOOST=0`, ridotti con `sips` a 1208×755 le tattiche e 1400×875 le scene, JPEG 82). Perché si vedano squadre, case e auto, il primo stato dà gli ordini: `name=ordini,t=1,pause=1,move=0:5300:4600+1:5000:4750+2:5450:4750+3:3600:3700+4:3900:4000,evac=Le Terrazze`. Con gli ordini: `tattica-squadre` (`t=40,pause=1`), `hero` (`t=80,at=district:Le Terrazze,dist=1200,yaw=120,pitch=-20`), `scena-borgo` (il sole della sera tra i tronchi, gli alberi che bruciano sul crinale: `ROCCA_START_HOUR=16.62 KIOSK_QUALITY=alta`, `t=80,at=4840:4470,dist=515,yaw=90.0,pitch=13.84`; camera a 4 m da terra, posizione scelta su una griglia di punti finché un albero copre il sole), `scena-cicatrice` (luce dell'alba del titolo: `phase=title,t=60,at=6700:5200,dist=2600,yaw=-74,pitch=-9`, `KIOSK_QUALITY=alta`). Galleria «Dal gioco» (2026-11-02, sei immagini `galleria-*`, 1200×750, JPEG 80; testi alternativi `g1`–`g6` in `tools/i18n/landing.py`): `carbonizzati` (come `scena-borgo`, camera in `5300:4550`), `versante` (`phase=title,t=80`, versante est all'alba), `torre` (`ROCCA_START_HOUR=16.36`, `t=105,at=3500:4197,dist=1001,yaw=89,pitch=2.4`), e con le squadre `autobotti` (`t=40,at=district:Le Terrazze,dist=1100,yaw=60,pitch=-20`), `pendio` (`t=80,at=team:0,dist=800,yaw=40,pitch=-22`), `tornanti` (`t=120,at=team:3,dist=900,yaw=30,pitch=-20`). Senza ordini: `tattica-isola` (`t=90,pause=1`) e `tattica-evacua` (`t=90,pause=1,select=Le Terrazze,at=district:Le Terrazze,dist=4200`). Le scene vanno con `ui=0,marks=0,iso=0`. Per il controluce la camera non deve finire dentro la collina: la posizione si controlla prima sul DEM. Le god rays fanno fasci solo se qualcosa copre in parte il sole (una chioma, un tronco, la torre), meglio da una camera bassa tra gli alberi; non con il vento da nord-est che porta il fumo verso la camera. Con `KIOSK_SNAP` la camera può guardare in su (`pitch` fino a +34°), le god rays restano accese con `KIOSK_GFX=-dof`, e `marks=0` nasconde anche le linee tagliafuoco gialle e gli anelli degli ordini. Con `KIOSK_SNAP` i pannelli del gamepad restano nascosti anche se un gamepad è collegato. Provata solo come pagina statica (desktop e 390 px), non con la build wasm.

## Dati separati dal codice e profilo grafico (2026-10-19)

**Wasm più piccolo (2026-10-30, iPhone che si ricarica):** modelli (`assets/models/*.json`, 10,7 MB) e immagini (personaggi, icone, logo) non sono più compilati nel `.wasm` per il web: viaggiano con i dati (`build_web.sh`, `datafs::take`, che libera anche la copia della pagina dopo la lettura); il desktop li tiene compilati dentro. Profilo `wasm-release` con `opt-level = "z"` e LTO completo; `wasm-opt -Oz` nella CI (binaryen version_133, checksum verificato). `.htaccess` con compressione (`mod_deflate`). Misure locali senza `wasm-opt`: `.wasm` da 49 a 29 MB (codice 31,3 → 22,5 MB, dati 19,3 → 6,7 MB), compresso da 12,5 a 8,6 MB. WebKit headless (`playtest_mobile.py --browser webkit --smoke --reloads 2`): parte, memoria lineare 258 MiB al titolo e 275 MiB nel tutorial (misura non confrontabile con quelle del 2026-10-21, fatte su un'altra build). **Non provato su iPhone.**

**Server lento (2026-10-30):** games.cimafoundation.org serve circa 500 KB/s, non comprime (l'`.htaccess` con `mod_deflate` è ignorato) e in Chromium headless il download HTTP/3 del wasm da 25 MB si è interrotto. `build_web.sh` scrive anche `game_bg.wasm.gz` (8,6 MB): la pagina lo scarica e lo decomprime in streaming con `DecompressionStream` (il `.wasm` pieno resta per i browser senza), e riprova una volta se il download si interrompe. Smoke test WebKit e Chromium passati in locale. Ogni download (dati, `game.js`, wasm) si riprova fino a 3 volte («TypeError: Load failed» su iPhone). Il proxy del server (FireShield, `x-vc-cache`) teneva in cache `index.html` e `data/manifest.json` per circa 15 minuti: dopo un deploy un iPhone emulato (WebKit) ha avuto il gioco nuovo con l'elenco dati vecchio («assets/models/town.json is not in the page's data»). Ora l'URL del manifest ha l'hash e l'`.htaccess` chiede `Cache-Control: no-cache` per HTML e JSON. **Aperto:** velocità e stabilità del server (Ergonet), da segnalare o da spostare su una CDN.

**Memoria iniziale da 320 MiB (2026-10-31, iPhone 13 con Chrome: la pagina muore appena dopo il logo, anche con i download riusciti).** Il wasm partiva con circa 1 MiB di memoria lineare e la allargava a piccoli passi fino a 260–280 MiB mentre costruiva la scena. Su iOS WebKit può dover ricopiare tutta la memoria a ogni allargamento, e nel picco ne servirebbe il doppio. Ora il linker la riserva subito (`.cargo/config.toml`, `--initial-memory`): non serve allargarla durante l'avvio. Le pagine non toccate non occupano RAM. Pubblicata e verificata sul sito (il wasm scaricato dichiara 320 MiB): **sull'iPhone muore ancora**.

**Registro dell'avvio (2026-10-31, tolto il 2026-11-02 su richiesta dell'utente: «spegni quello che non è necessario»).** La pagina scriveva in `localStorage` (`rocca_avvio`) i passi dell'avvio e gli ultimi messaggi della console, e dopo una pagina morta li mostrava in cima alla pagina introduttiva. Primo e unico esito dall'iPhone 13 (iOS 27.0.1, Chrome 156): `play`, `wasm`, `data`, `init`, poi `run 2 320 MiB` e la morte. Per un telefono senza Web Inspector va rimesso dalla storia git.

**Simulatore iOS 18.4 (Safari, Mac), `footprint` dei processi WebKit (2026-10-31):** a regime la pagina (WebContent) occupa 535 MB e la GPU 150 MB; **all'avvio la pagina sale a 1,9–2,1 GB per 1–2 s**, prima che la GPU salga. La sola compilazione del wasm, senza gioco, fa lo stesso picco (1,8 GB, poi 147 MB). Il codice sono 102 mila funzioni per 22 MB, nessuna sopra 232 kB. Il simulatore non ha i limiti di memoria né i thread dell'iPhone: indizio, non prova.

**Ricerca della causa sul telefono (2026-10-31).** Sull'iPhone 13 la sola compilazione del wasm, senza gioco, regge; l'isola spoglia parte; il colpevole è la **vegetazione** (indicazione dell'utente). Con il profilo del telefono sono 4197 piante, 0,11 M triangoli (case 0,14 M, terreno 0,35 M): non è la quantità di poligoni. Togliere allo shader delle piante la luce attraverso le foglie, lo stato del fuoco o la trama non basta; con un quinto delle piante (`?gfx=veg=0.005`) l'isola si vede e la pagina muore poco dopo, a 2 frame al secondo. Lo strato dei resti bruciati, costruito alla prima pianta che brucia (l'incendio della demo arde già sul titolo), sul web era una **mesh vuota** (0 vertici, 0 indici, visto nella console WebKit; il Mac la tollera): evitata quella, la pagina muore lo stesso, anche senza le mesh del fuoco e senza suoni. La console finisce con la scena costruita (13,4 s) e nessun errore, poi la pagina muore in silenzio (processo ucciso da iOS, probabilmente per memoria).

**LOD delle piante senza riscritture di mesh (2026-10-31).** Prima ogni blocco di piante passava tra i due livelli delle proxy (oltre `FAR_TIER_MIN_M` = 2000 m sul web) **scambiando la lista degli indici della sua mesh**: Bevy la riestraeva e la ricaricava intera sulla GPU. Con la camera del titolo che deriva, in WebKit (iPhone emulato, profilo telefono) erano **136 cambi di livello in 40 s**, ognuno una ricarica. Ora il livello lontano è una **terza mesh statica** per blocco (solo i vertici che usa, 0,05 M contro 0,10 M delle proxy) e il LOD cambia solo la `Visibility` delle entità (`vegetation::lod`). Le mesh delle proxy restano solo sulla GPU (`RENDER_WORLD`, indici a 16 bit): sparisce la copia CPU nella memoria wasm (circa 6 MB col profilo telefono), la GPU ne aggiunge circa 2,5 MB. Verificato con un log temporaneo degli `AssetEvent::<Mesh>::Modified` in 30–40 s di titolo: nessuna mesh delle piante modificata. **Restano riscritte a ogni frame tre mesh di `fire_view`** (fiamme, fumo, scintille: `meshes.insert`), più una manciata una sola volta. Scartati: `VisibilityRange` di Bevy (distanza dal centro del blocco e non dal punto più vicino, senza isteresi; la dissolvenza disegna due livelli insieme e vuole il dithering nello shader della vegetazione; su WebGL2 un buffer uniforme di 64 intervalli) e un LOD nel vertex shader (i vertici di dettaglio si elaborano comunque, servono vertici duplicati dove i livelli si toccano). **Da provare sull'iPhone.**

**Vegetazione riscritta (2026-11-01, branch `piante-ios`).** Le piante non sono più un `Material` di Bevy ma una pipeline propria (`crates/game/src/vegetation/`): sul telefono un riquadro solo, 33 varietà, 33 draw call instanziate, proxy opache, nessun `discard`, nessuna lettura di texture nel vertex shader (lo stato del fuoco è un attributo d'istanza). Misure in WebKit headless (iPhone, 390 × 844 a 3×), 120 frame del titolo: con le piante +33 `drawElementsInstanced`, +1 `useProgram`, circa +300 `vertexAttribPointer` per frame, **nessun caricamento in più** (325 KiB e 15 buffer creati e cancellati a ogni frame anche senza piante, soprattutto `fire_view`), due shader compilati all'avvio e nessuno dopo; 94 e 79 draw call per frame (titolo, tutorial), memoria wasm 320 MiB. Simulatore iOS 18.4, Safari, titolo per 170 s: WebContent 482–544 MiB e GPU 134–150 MiB dopo l'avvio, nessun riavvio del processo; senza piante (`?gfx=veg=0`) 508–536 e 131; il picco d'avvio di 1,9 GB c'è uguale con e senza piante (compilazione del wasm). **Non provato su un iPhone vero.**

**Memoria sull'iPhone: buffer GL rifatti a ogni frame (2026-11-01, branch `piante-ios`).** Sull'iPhone 13 (iOS 27, Safari) la pagina arrivava a 1241 MB di `phys_footprint`, circa 780 MB fuori dai tag VM (memoria grafica addebitata alla pagina), 1 fps e poi la chiusura; uguale senza piante. Contando le chiamate WebGL2 (WebKit headless, 390 × 844, profilo `q=bassa`, 120 frame) a ogni frame si creavano e cancellavano **17 buffer GL** (502 KiB) e 2–4 scritture parziali finivano negli **slab di indici condivisi** (8–12 MB) che la GPU sta ancora leggendo: il backend Metal del browser può rispondere copiando tutto il buffer. Le fonti:

- fiamme, fumo e scintille (`fire_view`) erano `Mesh` nuove a ogni frame: ora sono **mesh a capienza fissa riscritte sul posto** (`streamed_mesh.rs`: indici fissi per forma, lingua o billboard; vertici scritti con un `write_buffer` nel buffer che l'allocatore di Bevy ha dato alla mesh; zeri, cioè triangoli nulli, nello spazio non usato; la mesh si rifà solo se le forme non ci stanno, al doppio). Stesso metodo per i getti d'acqua (`units::animate_spray`) e per i colori delle case colpite (`buildings::damage`, solo i vertici delle case cambiate: prima tutto il blocco, 1,9 MB più gli indici, 5 volte al secondo);
- sei materiali delle isole lontane ricolorati a ogni frame (`horizon::tint`, ora solo se il colore cambia di mezzo passo a 8 bit), due materiali degli anelli pulsanti che nessuna entità usava (tolti), cielo e mare (`sky.wesl`, `water.wesl` prendono la direzione del sole dalla luce della scena e le nuvole si spostano da sole con `globals.time`; il materiale si riscrive solo quando colori o soglie cambiano: sul web di mezzo passo a 8 bit, sul desktop a ogni cambio come prima; l'effetto della pausa sul cielo è passato da `tactical::effects_off` a `tactical::sky_effects`, applicato prima del confronto);
- Bevy: `prepare_clusters_for_cpu_clustering` fa due uniform da 16 KiB nuovi per camera a ogni frame; sull'isola non ci sono luci a cluster, quindi sul web gira solo per una camera che non li ha ancora o se compare una luce a cluster (`web_gpu.rs`);
- sul web gli indici di ogni mesh hanno un buffer loro (`MeshAllocatorSettings::large_threshold` 0, `web_gpu.rs`): una mesh cambiata (lo strato del fuoco a terra, circa 7 volte al secondo in partita) è un buffer piccolo nuovo, mai una scrittura in uno slab condiviso. wgpu vuole quei buffer multipli di 4 byte: un numero dispari di indici a 16 bit riceve un triangolo degenere in più (`even_u16_indices`; senza, la pagina andava in panico all'avvio).

Risultato (stesse condizioni): titolo **17,5 → 0,22** buffer creati per frame (502 → 2,8 KiB), partita a ×300 col fuoco **15,3 → 0,23** (456 → 2,7 KiB); scritture `bufferSubData` 36 → 19 (675 → 312 KiB) al titolo, 32 → 17 (760 → 426 KiB) in partita; nessuna scrittura negli slab condivisi; buffer vivi 650 → 875 (gli indici separati), 50 → 47 MiB, stabili. Il contatore `hal.buffers` di wgpu continua a scendere di circa 16 per frame: sono le copie di appoggio di `write_buffer`, sul WebGL vettori nella memoria wasm creati senza contarli, non buffer GL. Aspetto: titolo web identico al prima (WebKit), desktop controllato con `KIOSK_SNAP` (fiamme, scintille, cielo e nuvole, case che bruciano).

Simulatore iOS 18.4 (Safari, `q=bassa`, `footprint`): titolo WebContent 508 → 512 MiB, GPU 141 → 139, 41,8 → 40,6 fps; partita 517 → 518 MiB, GPU 149 → 142, 25 → 24 fps. **Il simulatore non addebita alla pagina la memoria grafica come l'iPhone**: lì non si vede il guadagno, che resta da misurare sul telefono.

- **Risoluzione:** il canvas disegna già un pixel per pixel CSS (393 × 659 sul simulatore, densità 3): i bersagli di rendering sono piccoli, niente da limitare. Sul web non si carica più il logo da 1200 px dell'apertura (4,4 MiB di texture più la copia CPU che egui vuole): con un pixel per pixel CSS quello da 360 px basta.
- **Memoria wasm iniziale:** resta 320 MiB. Una `WebAssembly.Memory` da 320 MiB nel simulatore costa 19 MB finché non si tocca (200 MiB toccati: 219 MB); la partita intera (titolo, partita, finale col confronto, nuova partita) tocca al massimo 228 MiB prima e 238 MiB dopo (pagine non nulle; quelle azzerate ma allocate non si vedono), e la memoria non cresce mai. Scendere a 256 MiB non toglierebbe RAM e rischierebbe una crescita, cioè una copia, sul telefono.
- **Ricarica nella stessa scheda:** nel simulatore la pagina vecchia resta in memoria (WebContent 520 → 860–890 MiB per almeno 40 s) finché il garbage collector non la libera. All'uscita (`pagehide`) la pagina ora perde apposta il contesto WebGL (`WEBGL_lose_context`) e, uscendo, non ne chiede il ripristino (`web/index.html`). Ricarica dopo 15 s di titolo, WebContent 13 s dopo: con questo la memoria vecchia si è liberata in 3 prove su 6 (470–490 MiB), senza in 1 su 7; quando non si libera, il processo GPU resta circa 70 MiB più alto (213–221 contro 140–157). Con `preventDefault` sul `webglcontextlost` anche all'uscita, 0 su 2 e GPU a 222–230. Risultato incerto: **da verificare sull'iPhone** (ricaricare due volte e guardare `phys_footprint_mb` nel log), e da togliere se lì non aiuta. Dalla cache delle pagine la pagina riparte da capo.
- **Altro:** `window.roccaData` conserva 81 file, 5,4 MB gzip, letti di nuovo a ogni nuova partita; le copie CPU rimaste sono 21 MiB di mesh (case, che servono ai colori, segni) e 13,5 MiB di immagini (ritratti e logo per egui, LUT di Bevy). I 22 `AudioBufferSourceNode` al secondo sono di cpal (uno per blocco audio), non del gioco.

Da provare sull'iPhone: `http://192.168.1.82:8766/`. **Non provato su un iPhone vero.**

**iPhone: in HTTP Safari spegne il JIT (2026-11-01).** Sull'iPhone 13 (iOS 27.0.1, Safari), una pagina servita in **HTTP** gira nel processo `com.apple.WebKit.WebContent.EnhancedSecurity`, che non ha il JIT: JavaScript e WebAssembly vanno nell'interprete. Misure sul telefono, con il banco del Mac in `http://192.168.1.82`:

- un ciclo di 10⁷ somme richiede **136 ms in JS e 100 ms in wasm**, contro 10 e 5 ms nel simulatore iOS 18.4 sul Mac;
- la scena si costruisce in **38 s** (2 s nel simulatore) e il titolo gira a **0,7–2,4 fps**;
- il rapporto CPU del telefono (`cpu_resource`, 97 % di CPU per 90 s) mostra lo stack del wasm come la stessa funzione di JavaScriptCore ripetuta per ogni chiamata: l'interprete.

WebKit attiva «Enhanced Security» per le navigazioni HTTP verso host non locali; gli indirizzi privati come 192.168.x.x non sono esenti ([WebKit PR 78725](https://github.com/WebKit/WebKit/pull/78725)). Il processo resta del gruppo della pagina: una navigazione da script verso una pagina HTTPS (GitHub Pages) è finita nello stesso processo senza JIT. Il limite di memoria della pagina è 1536 MB, con o senza Enhanced Security (`runningboardd`).

**Conseguenza:** in HTTP l'iPhone con Safari è decine di volte più lento. GitHub Pages è in HTTPS, e dal 2026-11-02 anche games.cimafoundation.org (certificato Let's Encrypt valido, redirect HTTP → HTTPS fatto dal server). Il 2026-11-02 l'utente ha provato la build `8bc95c3` da GitHub Pages: «funziona benissimo».

**Non verificato:**

- il gioco in HTTPS sull'iPhone, con il JIT acceso: fps e memoria all'avvio. Nel simulatore la compilazione del wasm con il JIT arriva a 1,9 GB per 1–2 s;
- Chrome su iPhone: se usa la stessa regola.

Il banco con un certificato locale non ha funzionato: dall'iPhone non arrivava più nessuna connessione al Mac, né HTTP né HTTPS, causa non trovata.

**CPU per frame (2026-11-01).** Misure ripetibili con `scripts/perf_web.py`, headless e senza finestre:

- `frames`: CPU di ogni callback di frame in WebKit;
- `cpu`: profilo di Chromium per sistema Bevy;
- `gl`: chiamate WebGL e draw per programma.

Per i nomi delle funzioni nel profilo serve un wasm compilato con `--profile wasm-prof --config 'profile.wasm-prof.inherits="wasm-release"' --config 'profile.wasm-prof.strip="none"'`. Interventi, a parità d'immagine:

- **fiamme** (`fire_view::update_flames`, il 29 % della CPU del titolo): le righe di ogni lingua vengono da una tabella e le onde sono somme di angoli. Sono 3 `sin_cos` per lingua invece di 27 seni e 9 potenze di `libm`, che nel wasm costano. I vertici si scrivono già nell'ordine della mesh, con una copia sola (`QuadBuilder`, `streamed_mesh::put`). Stessa geometria: test `tongue_rows_match_the_direct_formula`;
- **niente metadati per mesh su WebGL2** (`web_gpu.rs`). Una mesh caricata senza copia CPU riceve `final_aabb`, e Bevy le dà un buffer di metadati suo. `prepare_mesh_bind_groups` creava così tre bind group per mesh a ogni frame: ~700 per frame, l'8 % del titolo. I metadati servono solo ai vertici compressi, che il gioco non usa;
- **`kiosk::view::fire_extent`** (case × punti del fronte, a ogni frame) si ricalcola solo quando cambia lo stato del gioco.

CPU per frame in WebKit headless, profilo telefono, media di due misure da 10 s, stesso profilo di compilazione (`opt-level = "z"`):

| | titolo | partita |
| --- | ---: | ---: |
| prima | 7,2 ms | 6,4 ms |
| dopo | 4,3 ms | 4,4 ms |

Chiamate WebGL sul titolo: da 3571 a 3009 per frame, per 77 draw call.

- **Scartato:** `opt-level = "s"` (wasm compresso da 8,7 a 10,0 MB) toglie solo un altro ~5 %; con `opt-level = 3` (12,4 MB) le fiamme erano 4 volte più veloci prima della copia unica dei vertici.
- **Restano:** le 33 draw call delle varietà di piante (in Chromium ~0,1 ms per frame), il passo della simulazione, egui e il backend GL di wgpu. Quest'ultimo riconfigura attributi, texture e sampler a ogni draw: circa 40 chiamate WebGL per oggetto.

**Dati.** I file del territorio (`data/scenarios/rocca_ventosa`, `data/behaviours`, `fuels_eu12.json`; dal 2026-10-30 anche `data/sounds`, 0,65 MB di MP3) non sono più nel `.wasm`. `build_web.sh` li comprime in gzip in `target/web/data/` e scrive `data/manifest.json` (percorso, URL con hash del contenuto, byte). La pagina scarica in parallelo il `.wasm` (compilato in streaming) e i file, li mette in `window.roccaData` e solo allora avvia il gioco; `datafs` li legge di lì (`crates/datafs`). Se il browser ha `DecompressionStream` (Chrome 80, Safari 16.4, Firefox 113) decomprime lui; altrimenti i byte restano gzip (`window.roccaDataGzip`) e li decomprime il wasm (`flate2`, a ogni lettura). Ogni file sta in Cache Storage (`rocca-data`) con l'URL con hash: una nuova build del codice non fa riscaricare i dati, e viceversa. Dati: 27 file, 4,5 MB gzip (15,9 MB grezzi). Con la decompressione nel browser restano in memoria JS circa 16 MB; con quella nel wasm solo i 4,5 MB compressi. Le due strade non sono state provate entrambe su un browser vecchio.

**Profilo.** La pagina sceglie `window.roccaProfile` e `quality::Graphics::from_env` lo legge: **`bassa`** (`Graphics::WEB`, il profilo di sotto) su telefoni e tablet (puntatore primario grossolano, `(pointer: coarse)`, o `userAgentData.mobile`); **`media`** (`Graphics::WEB_MEDIA`) su computer: come `WEB` più ombre del sole (2 cascate, 1024), terreno a passo 2 e vegetazione 0,08 (0,05 fino al 2026-11-02). `?q=bassa` o `?q=media` forzano il profilo, per le prove; `?gfx=` lo corregge come `KIOSK_GFX` sul desktop (es. `?gfx=veg=0,terrain_stride=8`). I tetti del fuoco, il mare e la risoluzione del suolo restano uguali nei due profili. Dal 2026-10-21 su computer il livello è **adattivo** (`adaptive.rs`, vedi `TODO.md`): parte da `WEB_TOP` (`WEB_MEDIA` con ombre da 3 cascate, 2048, e vegetazione 0,14) e scende se i frame sono lenti; `?q=` lo spegne. Verificato in Chrome su desktop con entrambi i valori di `?q=` (parte, grafica a vista); **non misurati gli FPS** (dal 2026-10-20 il terreno web usa colore e normale per vertice, senza le texture `ground_*`: WebGL2 ha 16 unità texture, vedi `TODO.md`) e non provato su un telefono vero.

## WebGPU sui computer (2026-11-02)

Richiesta dell'utente: «portare web sulla stessa qualità di desktop». WebGL2 non ha target HDR con MSAA, ha la profondità di OpenGL, 16 unità texture e un buffer GPU per ogni mesh cambiata: da lì venivano quasi tutte le rinunce del web. Bevy decide il backend alla compilazione (la feature `webgpu` sostituisce `webgl2`), quindi le build sono due, dallo stesso codice:

- `build_web.sh` compila `game.*` (WebGL2) e `game_gpu.*` (`--features webgpu`, cartella `target/webgpu` perché le due non si ricompilino a vicenda); `ROCCA_WEB_GPU=0` fa solo la prima. Ogni wasm ha il suo hash nell'URL e la sua cache (`rocca-wasm`, `rocca-wasm-gpu`).
- La pagina carica `game_gpu` se il profilo è quello dei computer (`media`) e `navigator.gpu.requestAdapter()` dà un adattatore; altrimenti WebGL2. `?gpu=0` tiene WebGL2, `?gpu=1` chiede WebGPU anche sul telefono.
- **Ritorno a WebGL2:** se il renderer si ferma su un errore (`RenderErrorHandler` in `webgpu_shaders.rs`) o il wasm va in panico, la pagina si ricarica con `?gpu=0` ed entra nel gioco da sola.
- Nel codice i limiti di WebGL2 stanno dietro `quality::WEBGL2` (vero solo nella build WebGL2): HDR, MSAA, god rays, contorno di Mario, piante a riquadri con resti e modelli autoriali, maschera delle strade a 2 m, mare a 15 m e largo a 150 m, tetti del fuoco, `web_gpu` e `web_near_plane` (questi due solo su WebGL2). Le cose del browser in sé (un thread, `localStorage`, dati scaricati) restano su `target_arch = "wasm32"`.
- **Livelli:** con WebGPU il profilo `media` parte da `Graphics::ALTA`, come il desktop, e la grafica adattiva scende; `?q=media` ferma «media», `?q=bassa` «bassa».

**Il compilatore WGSL del browser** (Tint in Chrome) è più severo di naga, e il primo avvio si fermava:

- gli shader PBR di Bevy 0.20 (`pbr_fragment`, `pbr_prepass`, `pbr_prepass_functions`) mettono texture e sampler in un `let`, che WGSL vieta (ancora così su `main` di Bevy): `webgpu_shaders.rs` li riscrive quando arrivano tra gli asset, prima che una pipeline li compili, con le texture legate al posto del nome (in WebGPU non ci sono texture bindless); un test lo controlla sui sorgenti di Bevy nel registro di Cargo;
- le derivate (`dpdx`, `fwidth`) dopo un ramo su un valore non uniforme: nella sola build WebGPU gli shader con un `@fragment` ricevono `diagnostic(off, derivative_uniformity);` dopo gli `import` (`shader_lib::derivatives_anywhere`).

**Misure** (Chromium headless su Mac M4 Pro, Metal, 1440×900 CSS; titolo e partita dopo 30 s; 600 frame):

| | WebGL2 | WebGPU |
|---|---|---|
| titolo, 1× | — | 10,6 ms (94 fps) |
| titolo, 2× (2880×1800) | 8,5 ms | 29,2 ms (34 fps, la grafica adattiva era scesa di 2 gradini su 16) |
| partita, 1× | — | 8,3 ms (120 fps) |
| partita, 2× | 8,5 ms | 11,3 ms (89 fps) |
| piante | 23 158, 1,5 M triangoli | 83 117, 7,4 M triangoli |
| terreno costruito in | 2,5 s (passo 2) | 6,4 s (passo 1, pagina bloccata) |
| memoria (RSS) renderer / GPU | 783 / 236 MiB | 1292 / 528 MiB |

A 2× il costo sono i pixel (con `gfx=scale=0.5` 8,5 ms): è il caso della risoluzione adattiva. Immagini a confronto, stesse inquadrature (`perf_web.py shots`, `?gpu=1` contro `?gpu=0`): ombre del sole, alberi autoriali con le foglie, tronchi bruciati, bloom, profondità di campo, erba vicina. Nessun errore in console; prova `--smoke` del telefono passata in Chromium e WebKit (WebGL2, come prima). WebKit di Playwright non ha WebGPU: Safari 26, che lo ha, non è provato.

## Stile realistico e volumi nella build WebGPU, in prova (2026-11-06)

Richieste dell'utente: «anche su web dovremmo avere scansioni ed effetti volumetrici se possibile», poi «porta le modifiche su web». È possibile nella build WebGPU; restano **spenti finché non li chiede l'indirizzo**, come sul desktop le variabili `KIOSK_SCANS` e `KIOSK_VOLUMI`:

- `?volumi=1`: fiamme e fumo volumetrici (`fire_volumetric::on`). Gli shader passano il compilatore del browser senza modifiche, compresa la ricomposizione nei campioni (`sample_mask`) con l'antialiasing a 4 campioni.
- `?fluido=1` (con `?volumi=1`): le fiamme vicine alla camera da un fluido 3D in compute shader (`fire_fluid.rs`, 2026-11-07, in prova). Storage texture `rgba16float` e `r32float` in sola scrittura, che WebGPU ha di base. Provato in Chromium headless (1440×900 a 2×, `?q=media`, minuto 90): nessun errore né ritorno a WebGL2, +1,6 ms a 250 m dal fronte e +2,0 ms a 120 m (13,5 → 15,1 e 18,5 → 20,4 ms per fotogramma); il fronte anticipato costa ≤ 0,4 ms. In WebGL2 non c'è (niente compute shader). Guardato (stessa build, 1440×900, partita alle 12:30 in pausa, cinque viste dal fronte a 45–300 m, con e senza): fiamme come sul desktop, lingue più mosse e più larghe alla base con il fluido; da 300 m quasi nessuna differenza. Per le foto, `roccaPlaytest.hide = 1` toglie zone della previsione, segni e interfaccia, come `ui=0,marks=0,iso=0` di `KIOSK_SNAP`; `roccaPlaytest.state.front` dà una cella che brucia.
- `?anticipo=0`: il bordo del fuoco senza il fronte anticipato (`fire_ahead.rs`, acceso in entrambe le build; nel browser il fork del nucleo gira nel fotogramma, ogni 60 s simulati invece di 12).
- `?scans=1`: lo stile realistico (`models::scans`), cioè le piante scansionate e, con lo stesso interruttore, il terreno fotografico, i ciuffi d'erba vicini, gli edifici e le rocce (`textures::ground_detail`, `materials/masonry.rs`). Nel browser i file si leggono con `datafs::take` dagli stessi percorsi del desktop. `build_web.sh` ne prepara una versione per il web:
  - piante: **senza il livello più fine** (`lod0`, tre quarti dei byte: da vicino si vede `lod1`) e con i cartoncini a metà lato (2048 px): 8,3 MB compressi;
  - terreno (`assets/ground/`: nove strati, `ground.json`, i ciuffi) ed edifici (`assets/buildings/`: intonaco, coppi, pietra, roccia, colore e normali): ogni PNG **a metà lato** (512 px; il codice legge le dimensioni dai file): 5,7 e 4,4 MB compressi.
  - In tutto **18,4 MB compressi in 22 file**, con `"only": "scans"` nel manifest: la pagina li scarica solo con `?scans=1` nella build WebGPU; senza, il sito scarica gli 11,8 MB di prima. Le riduzioni usano Pillow: dove manca (la CI di `pages.yml` oggi non lo installa) lo script lo dice e il sito esce senza stile realistico.
- Su WebGL2 (telefoni) niente di tutto questo: lì i binding in più non esistono (16 unità di texture) e l'erba vicina resta quella di prima.

Prova (Chromium headless, Mac M4 Pro, 1440×900, partita a 120 minuti):

- `?gpu=1&volumi=1&scans=1`, cioè la qualità automatica dei computer: backend `BrowserWebGpu`, antialiasing a 4 campioni, grafica adattiva al livello 3 di 16, nessun errore in console, nessun ritorno a WebGL2, i 22 file richiesti, «realistic ground: 9 layers of 512² and grass cards of 512 x 256», 171 mesh di piante, 120 fotogrammi al secondo a 300 m dal fuoco (il passo dello schermo). Lo stesso con `?q=media`.
- Senza i parametri, `?gpu=1` e `?gpu=0`: nessuna richiesta dei file in più, nessun errore, 120 fotogrammi al secondo.
- Immagini guardate: vista di gioco, fronte a 900 e 300 m, rocca, torre, pineta, chiesa, borgo.

**Non provato:** su un computer più lento, su Safari e Firefox, a 2×; la memoria della pagina con lo stile realistico; il sito pubblicato (serve Pillow nella CI, oppure i file ridotti nel repository); una vista bassa con le foglie contro il fumo nel browser (la correzione del contorno è stata guardata sul desktop: lo shader è lo stesso).

## Vegetazione più fitta, a lobi (2026-11-02)

Richiesta dell'utente: «più densità e resa grafica, senza perdere performances possibilmente» (branch `piante-ios`).

- **Piante a lobi** (`vegetation::meshes::lobed_mesh`; `Graphics::lobes`, acceso nei profili web; `?gfx=-lobes` torna alle proxy per i confronti). La proxy di prima era una chioma a sei facce sul riquadro delle foglie del modello, con la tessera della chioma stirata e specchiata faccia per faccia: da vicino un ombrello sfaccettato a raggiera, da sopra una stella. Ora ogni pianta è il modello autoriale senza le carte di foglie, che vorrebbero il ritaglio (su WebGL2 senza MSAA un `discard` a bordi seghettati, e le GPU a tile del telefono perdono la rimozione delle superfici nascoste): tronco e rami come sono, e ogni parte della chioma un grumo chiuso, il suo nucleo (un icosaedro irregolare) allargato all'82 % dell'ingombro delle sue carte (`LOBE_FILL`), nel tono delle carte. Un pino tiene i pad a ombrello sul tronco biforcuto, un leccio i suoi tre lobi. Opaco, una faccia, niente `discard` né blending: 65 triangoli per pianta in media (la proxy 28). Il desktop non cambia (`lobes` spento: le sue proxy restano oltre `lod_m`).
- **La tessera dipinta dallo shader** (`PLANT_CANOPY` in `plants.wesl`): la tessera della chioma della specie ripetuta sulla pianta (6 volte sulla sua altezza) in tre proiezioni sfumate con la normale, specchiata a ogni ripetizione (niente cuciture), mip scelte dal gradiente della coordinata non ripiegata (`textureSampleGrad`). Coordinate sulla pianta ferma, prima del vento (le foglie non scorrono sulla chioma che ondeggia), spostate per pianta. Tre letture di texture per frammento di chioma, nessun attributo in più per vertice oltre la coordinata.
- **Chiome bruciate:** senza i resti (sul web non ci sono) la chioma carbonizzata resta ristretta al 40 % verso il tronco (`plant_common.wesl`): un ciuffo nero tra i rami nudi, non più una palla nera grande come la chioma viva.
- **Densità:** telefono (`WEB`) 0,025 → 0,05 (4197 → 8366 piante); computer con `?q=media` 0,05 → 0,08 (8366 → 13 283); cima della scala adattiva dei computer (`WEB_TOP`) 0,10 → 0,14 (16 700 → 23 400). Costruzione della vegetazione in Chromium: 20–40 ms prima, 30–45 ms dopo.
- **Il diradamento risparmia vertici** (`PlantScene::thin_by_count`, solo web): ogni gruppo è ordinato per il rango del diradamento e si disegnano solo le piante tenute (`Group::kept`). La grafica adattiva, che dirada al 60, 35 e 20 % (anche sul telefono), ora toglie davvero vertici: prima lo shader scartava le piante diradate dopo averne calcolato i vertici. Unica differenza: sul web una pianta diradata non compare più quando brucia (sul desktop sì, come prima).
- **Ombre** (computer): i lobi proiettano la propria ombra.

Misure (headless, nessuna finestra; `scripts/perf_web.py`, ora con `--viewport`, `--dpr` e `shots`). CPU per frame in WebKit, 10 s dopo 25 s di assestamento, misure alternate prima/dopo (A/B/A/B; due per parte, quattro per il titolo sul computer), in ms, media / p95 (WebKit dà il tempo al millisecondo); il rumore tra due misure uguali è del 5–10 %:

| | telefono `q=bassa`, titolo | telefono, partita | computer `q=media` 1440×900, titolo | computer, partita |
| --- | ---: | ---: | ---: | ---: |
| prima | 4,56 / 8 | 4,39 / 6–7 | 5,56 / 9–10 | 5,20 / 7 |
| dopo | 4,50 / 8 | 4,36 / 6–7 | 5,67 / 9–10 | 5,26 / 7 |

WebGL per frame (Chromium, titolo, 120 frame): chiamate e draw invariati (telefono 2969 → 2982 chiamate, 75,5 draw istanziate; computer 4485 → 4486, 119), sempre 33 draw di piante per passata. Cambiano i vertici (indici × istanze): piante del telefono 0,35 → 1,63 milioni per frame, su 1,14 → 2,42 milioni dell'intera scena; computer `q=media` 1,39 → 5,17 milioni (passata principale e ombre). Memoria: per pianta 40 byte sulla GPU e circa 53 sulla CPU (record, fuoco, posizione, rango), cioè +0,2 e +0,2 MB sul telefono; le mesh a lobi 33 × ~2 KB. La memoria wasm resta quella riservata all'avvio (320 MiB).

Immagini prima/dopo alla stessa inquadratura: `roccaPlaytest.camera = {x, y, distance, yaw, pitch}` tiene ferma la camera con `?playtest=1` (`playtest::camera`, piazzata come `snap::hold`), `perf_web.py shots` la usa.

Provato e scartato:
- ombre dalla chioma più economica (`Tier::Far`, la bipiramide) per risparmiare vertici nelle cascate: spuntava dai lobi e li segnava di intaccature nere a spigoli dritti;
- la tessera una volta per lobo, dalle uv autoriali del nucleo: stirata, «plastica», come il nucleo grande del 2026-11-01;
- il tempo GPU con `EXT_disjoint_timer_query_webgl2` in Chromium (ANGLE su Metal): lo stesso con zero piante e con 10 milioni di triangoli, la misura non vede il lavoro della GPU.

Misurati per una decisione dell'utente, non accesi:
- **erba vicina** (`?gfx=cover=22`, profilo del telefono): ciuffi d'erba veri davanti alla camera nelle viste basse (titolo, zoom in partita), niente nelle viste alte; 1 draw e 442 mila vertici per frame quando c'è (quanto il terreno), +180 chiamate WebGL, una texture riscritta ogni 0,25 s; CPU per frame 4,28 e 5,60 ms contro 4,39 e 4,29 (titolo, la camera del regista va e viene dal suolo). `near_cover` è in modifica in un'altra sessione;
- **modelli autoriali vicino alla camera** (`?gfx=lod=1400`, computer): sul web l'isola è un riquadro solo, quindi tutte le piante passano alle carte ritagliate; senza MSAA il ritaglio è un `discard` a bordi seghettati e le chiome sembrano rade e scure; 3,55 milioni di vertici per passata contro 2,59 dei lobi. Senza l'MSAA (alpha to coverage, oggi spento sul web) non conviene.

**Non provato su un iPhone vero.** Il rischio è la GPU del telefono: i vertici per frame raddoppiano (opachi, senza `discard` né blending, a un pixel per pixel CSS). Se i frame passano i 24 ms la scala adattiva dirada le piante e ora ne toglie anche i vertici. Da provare in HTTPS con `?gfx=veg=0.025` (la densità di prima) e `?gfx=-lobes` (le proxy) per separare i due costi.

## Lingua (2026-10-21)

Il gioco segue la prima lingua supportata tra quelle preferite del browser (`navigator.languages`, anche per prefisso: `de-AT` → `de`; altrimenti inglese, 2026-10-30), dopo l'ultima scelta salvata in `localStorage`; `?lang=de` la forza (la pagina la passa al wasm come `window.roccaLang`). I cataloghi `data/i18n/*.json` sono nel manifesto dei dati come gli altri (`scripts/build_web.sh`; `keys.json` è escluso). Il pulsante delle bandiere del titolo c'è anche sul web.

**Pagina introduttiva tradotta (2026-10-30, richiesta dell'utente):** stessa regola del gioco: `?lang=` > scelta salvata (`rocca_ventosa_lingua`, condivisa con il gioco) > lingua del browser > inglese. Un selettore 🌐 nell'intestazione cambia lingua alla pagina e al gioco. Testi (anche caricamento, errori, nome, toast della ROM) in `tools/i18n/landing.py`, che genera `web/i18n.js` (`python3 tools/i18n/landing.py` dopo ogni modifica); «Gioca», «Come ti chiami?», «Continua» vengono dai cataloghi del gioco. La pagina passa sempre la lingua al gioco in `window.roccaLang`. Con `?lang=tlh` la pagina resta in inglese e il gioco va in klingon. Traduzioni automatiche, non riviste. **Non vista nel browser**: provata la logica con node (de-AT→de, ja→en, pt-BR→pt, scelta salvata, tlh).

## Risoluzione adattiva (2026-11-02)

Richiesta dell'utente: «la risoluzione adattativa per andare almeno a 30 fps sia in desktop che in web». La scena 3D si disegna a una quota della risoluzione dello schermo, per lato (`Graphics::render_scale`), e si allarga prima degli effetti: interfaccia, testi ed effetti restano a piena risoluzione. `render_scale.rs` mette sulla camera il `MainPassResolutionOverride` di Bevy (prepass, opaco e trasparente nell'angolo in alto a sinistra della vista) e subito dopo il main pass lo allarga a tutta la vista con un filtro bilineare (`Core3dSystems::EarlyPostProcess`, `shaders/upscale.wesl`).

- **Chi la muove:** la scala di `adaptive`, su desktop e web: 0,85 subito dopo la profondità di campo, 0,7 dopo le ombre ridotte, 0,6 senza ombre, poi 0,5 e 0,4 in fondo. Scende sotto i ~42 fps (mediana di 2 s), risale con una prova dopo 12 s sopra i ~55.
- **Una discesa che non accorcia i frame si annulla:** se tre gradini sotto l'ultimo che ha reso almeno l'8 % i frame sono lunghi come prima, il livello torna lassù e non si riprova finché i frame non peggiorano di un quarto. Il tempo va altrove (la CPU, o un browser che tiene la pagina a 30 Hz, come l'iPhone in risparmio energetico), e scendere ancora sfocherebbe e diraderebbe e basta. Tre gradini e non uno: la leva che aiuta può stare più in basso (a un Chromium senza GPU piante e risoluzione non davano niente; le ombre sì).
- **Macchine lentissime:** i frame oltre 0,25 s erano sempre «intoppi» (scheda nascosta, finestra trascinata) e non contavano; un Chromium senza GPU disegnava il titolo a 1 fps e il livello non si muoveva mai. Ora, se durano più di 2 s di fila, contano. Il controllore gira solo a partita caricata e riparte da capo a ogni nuova partita.
- **Web:** il canvas ha la densità dello schermo, al massimo 2× (`web_input::backing_scale`; prima 1×, un terzo per lato su un iPhone). I testi sono nitidi; la scena parte da 2× e scende se serve (0,5 equivale alla risoluzione di prima).
- **Sotto 1** la profondità di campo è spenta (legge la profondità a piena risoluzione), e con Mario la quota torna 1 (il contorno legge maschera e profondità a piena risoluzione).
- Il gradino salvato ha cambiato nome (`grafica_auto2.txt`, `rocca_ventosa_grafica_auto2`): la scala ha più gradini e un indice vecchio cadrebbe altrove.
- **Forzarla:** `KIOSK_GFX=scale=0.5`; sul web `?q=bassa&gfx=scale=0.5` (solo `?q=` spegne l'adattiva). `KIOSK_ADAPT=1` la tiene accesa anche con `KIOSK_SNAP`, senza toccare il gradino salvato.

Misure (M4 Pro, finestra nascosta 6400×4000, partita a T+150 sull'incendio, «alta» senza profondità di campo, `KIOSK_SNAP_BENCH=8`):

| quota | GPU | fps |
|---|---|---|
| 1 | 17,2 ms | 54 |
| 0,7 | 14,4 ms | 67 |
| 0,5 | 10,1 ms | 98 |

Con `KIOSK_ADAPT=1` a 7838×4898 il controllore ha spento la profondità di campo, portato la scena a 0,85 e diradato le piante, poi dopo 40 s di margine è risalito di un gradino: 48 fps di media.

## Telefono e budget grafico (2026-10-15)

La versione web usa un profilo leggero anche sui computer: terreno a 20 m,
vegetazione al 5 % (2,5 % fino al 2026-11-02) con le piante a lobi, opache e senza
carte di foglie (vedi «Vegetazione più fitta, a lobi»), mare a 60 m vicino
all'isola e 600 m al largo.
Niente ombre, HDR, profondità di campo, bloom o MSAA. Il canvas ha la densità
dello schermo, al massimo 2× (`web_input::backing_scale`; fino al 2026-11-02 un
pixel per pixel CSS, un terzo per lato su un iPhone), e la scena 3D vi si disegna
alla quota che il frame time permette (sezione «Risoluzione adattiva»); gli input
sono normalizzati in pixel CSS. Le mesh statiche di terreno e mare rilasciano la copia CPU
dopo il caricamento sulla GPU, anche nella versione locale, a geometria e resa
invariate. I ritratti conservano invece la copia CPU: bevy_egui richiede che
l'immagine resti negli asset per mantenere la texture registrata.
Il fuoco conserva al massimo 600 celle con fiamme,
100 particelle di fumo e 80 braci, con lo strato a terra a 20 m.
Sono semplificazioni visive: dati del territorio, simulazione, ordini e punteggio
restano quelli della versione desktop. Il menu web non permette di riattivare effetti pesanti.

L'interfaccia compatta si attiva sotto 1000 px di larghezza o 600 px di altezza:

- vista iniziale dall'alto (nadirale), nord in alto e zoom ampio sull'incendio; poi la camera è libera come sul computer (2026-11-03): due dita che scorrono insieme in su o in giù la inclinano;
- verticale: istruzioni in basso; orizzontale: tutorial a destra, mappa a sinistra;
- la schermata del titolo è quella del desktop (logo, titolo, sottotitolo, «Gioca»), in scala sulla larghezza;
- apertura, scelta dello scenario, finale e menu scorrono se non entrano nello schermo;
- il nome usa un campo HTML con tastiera del telefono;
- un dito sposta la mappa, due dita fanno zoom, rotazione e spostamento, oppure inclinano (dita affiancate che scorrono in verticale nello stesso verso; il gesto si decide dopo 12 px, `view::two_finger_tilt`);
- in pausa si trascina il badge della squadra, oppure si tocca la squadra e poi l'esagono;
- durante il trascinamento si vedono percorso, tempo o motivo del rifiuto;
  tenendo il badge vicino al bordo della mappa, la vista scorre in quella
  direzione, più rapidamente avvicinandosi al bordo;
  rilasciare su un pannello annulla l'ordine; una gesture annullata o con due
  dita non invia ordini;
- un tocco sul borgo apre la scheda con persone, tempo e «Evacua»;
- «+», «−» e «Isola» restano disponibili senza gesti o tastiera.

Vista nadirale e icone dei paesi verificate il 2026-10-15 con build web e
test WebKit `--smoke`: avvio, nome, tutorial e rotazione superati; PNG
verticale e orizzontale controllati (`out/playtest/mobile-nadir`).

Lo scorrimento durante il trascinamento usa gli ultimi 48 px della mappa
libera, con velocità progressiva e proporzionata allo zoom. Pannelli e
istruzioni sono esclusi; tornando al centro, rilasciando o annullando il
gesto lo scorrimento si ferma. Verificato con Chromium touch a 390×844,
densità 3×, sui quattro bordi, arresto al centro e sui pannelli, `touchCancel`
e rilascio sul pannello senza ordini (`scripts/playtest_mobile.py --edge-pan`,
PNG in `out/playtest/mobile-edge-pan`).

Le istruzioni e i messaggi vanno a capo; sul web mobile i borghi sono indicati
da icone di casa senza nome, disposte senza sovrapporsi e con un collegamento
al punto sulla mappa. Il nome compare nella scheda aperta al tocco. La pagina rispetta
le aree sicure del telefono e consente di cambiare orientamento senza ricaricare.
Il confronto «senza ordini» del finale avanza a piccoli blocchi (budget di 8 ms,
con un passo indivisibile), lasciando disponibili interfaccia e riavvio. La
previsione mantiene i 16 membri originali, uno per frame nel browser.

```text
./scripts/build_web.sh                                  # sito statico in target/web
python3 -m http.server 8765 --directory target/web      # http://localhost:8765
```

Con `CARGO_TARGET_DIR` impostato il sito va in `$CARGO_TARGET_DIR/web`. Da Bevy 0.20 (2026-10-21): Rust 1.97.1 (`rust-toolchain.toml`), `wasm-bindgen-cli` 0.2.129 (stessa versione di `Cargo.lock`, anche in `pages.yml`), `getrandom` 0.2 con `js` e 0.4 con `wasm_js` per il target wasm, `strip = "symbols"` nel profilo `wasm-release` (senza, il `.wasm` pesava 126 MB; ora 61 MB, `wasm-opt` non installato). Provato in Chrome headless solo con `q=bassa` fino al titolo.

Pubblicazione: `.github/workflows/pages.yml` costruisce e pubblica su GitHub Pages a ogni push su `main` o `settimana-protezione-civile` (`nuovo-gioco` è stato unito a `main` e cancellato il 2026-11-02). Indirizzo: https://cimafoundation.github.io/isola-di-roccavento/ (repository `CIMAFoundation/isola-di-roccavento` dal 2026-10-30); da `main` anche https://games.cimafoundation.org/isola-di-roccavento/

**games.cimafoundation.org/isola-di-roccavento** (2026-10-30): hosting CIMA su Ergonet (`web-lnx345.ergonet.host`, 149.3.144.47); la radice FTP dell'utente `cima-games` è la radice di games.cimafoundation.org (c'era la pagina segnaposto di Ergonet). Da `main`, `pages.yml` copia `target/web` con `lftp mirror --reverse --delete` in FTPS (porta 21, certificato `*.ergonet.host` verificato: per questo l'host è `web-lnx345.ergonet.host` e non l'IP) nella cartella `isola-di-roccavento` (variabile `DEPLOY_DIR` del repository per cambiarla). Secret: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_PASSWORD`. L'SFTP sulla porta 2201 accetta solo chiavi e non è usato. Il passo è `continue-on-error`: se fallisce, GitHub Pages esce lo stesso. Il server mandava `game_bg.wasm` senza tipo MIME e il gioco non partiva (2026-10-30): `build_web.sh` scrive un `.htaccess` con `AddType application/wasm .wasm` e `index.html` compila il file intero quando il tipo manca. Dal 2026-11-02 il sito risponde in HTTPS con un certificato valido (Let's Encrypt, scade il 2027-01-28) e il server reindirizza l'HTTP con un 301; dopo il push di `8bc95c3` serviva lo stesso wasm di Pages (il primo download subito dopo il caricamento era troncato, i quattro successivi interi).

**Versione provata (2026-10-15):** `crates/game/build.rs` registra il commit della compilazione (`GAME_COMMIT`, con `-modificato` se l'albero non è pulito; in CI è quello pubblicato). Compare sul titolo («versione xxxxxxxx») e il pulsante «Dai un feedback» lo scrive da solo nella domanda «Versione del gioco» del Google Form (`entry.2126742218`).

**Cache del download (2026-10-17):** il `.wasm` è salvato in Cache Storage (`rocca-wasm`) con chiave l'URL versionato (`?v=<hash>`). Finché la build non cambia, i caricamenti successivi su telefono non scaricano nulla; con una nuova build si scarica di nuovo e la copia vecchia viene cancellata. Se Cache Storage non c'è o fallisce, si scarica come prima. Non provato su telefono.

Serve un browser con WebGL2. Il primo caricamento resta di circa 53 MiB non compressi:
la riduzione principale riguarda la memoria e la grafica durante il gioco.
Una scheda in secondo piano viene sospesa dal browser. In caso di perdita del
contesto grafico compare un pulsante per ricaricare.

## Verifica touch del gioco attuale

Correzione Safari del 2026-10-15, dopo la segnalazione di «Gioca» non
cliccabile su iPhone: riprodotto con WebKit 26.0 a densità 3×. Il resize
del browser riportava 1170×2532 per un canvas CSS da 390×844, nonostante
l'override 1×; l'interfaccia e i tocchi finivano così su scale diverse.
L'adattatore web sincronizza la risoluzione con le dimensioni CSS del canvas
prima di elaborare gli input; il CSS conserva le dimensioni del contenitore
anche quando winit richiede un resize. La verifica WebKit con tocchi nativi
copre «Gioca», nome HTML, avvio e avanzamento del tutorial e rotazione:
superata, senza errori browser; PNG controllati. Non è una prova su iPhone
fisico né certifica specificamente iOS 27.

Regressione Chromium del 2026-10-17 (flusso con scelta dell'incendio): tutti i controlli
di interazione fino al finale e al riavvio superati, nessun errore browser.
Memoria: 295 MiB nel tutorial, 379–416 MiB al finale; il limite del test è 450 MiB (2026-10-17).
La crescita avviene durante la partita, non al confronto senza ordini; vedi `docs/TODO.md`.

```sh
.venv/bin/python -m playwright install webkit
.venv/bin/python scripts/playtest_mobile.py --browser webkit --smoke --output out/playtest/webkit
```

Verifica precedente del 2026-10-15: `scripts/build_web.sh`, `cargo check -p game` e la
regressione touch completa superati. Chromium headless su macOS, densità 3×:
390×844, 844×390, 320×568, 568×320 e mouse a 1280×800. Nell'ultima esecuzione
(24 s) nessun errore browser; memoria lineare Wasm circa 203 MiB nel tutorial
e 273 MiB al finale (295 MiB in un'altra esecuzione). Il test impone
un limite di regressione di 450 MiB. Vegetazione: circa 10.900 piante e 180.000
triangoli nel profilo web. PNG controllati per tutorial, evacuazione, partita,
finale e telefono piccolo.

Il test copre anche rilascio sul pannello e `touchCancel`: il badge torna al suo
posto e il gesto successivo funziona. La versione di bevy_egui usata lasciava
premuto il suo pulsante mouse virtuale dopo `touchCancel`; l'adattatore web lo
rilascia esplicitamente senza trasformare l'interruzione in un ordine.

```sh
scripts/build_web.sh
.venv/bin/python scripts/playtest_mobile.py
# Per usare un Chromium già installato:
.venv/bin/python scripts/playtest_mobile.py --browser-executable /percorso/chrome-headless-shell
```

Il test usa eventi touch, viewport 390×844 e 844×390 a densità 3×, poi 320×568 e 568×320.
Controlla anche il mouse a 1280×800.
Controlla nome con tastiera HTML, tutorial, selezione con due tocchi, drag-and-drop,
evacuazione, zoom, orientamento, finale e riavvio. Salva PNG, osservazioni e
`report.json` in `out/playtest/mobile`, più **`report.html`**: un file solo, con tutte le immagini incorporate nell'ordine del test, fase, passo e memoria di ciascuna, e l'esito (da aprire nel browser per ispezionare). La memoria registrata è la capacità della
memoria lineare Wasm, non la RAM totale del browser/GPU. L'emulazione desktop
non sostituisce una prova su Safari iOS e Chrome Android su dispositivi reali.


## Runner del vecchio flusso (2026-10-13)

La modalità automatica di `playtest.py` qui sotto si riferisce al vecchio gioco
e va aggiornata; per il flusso attuale usare `playtest_mobile.py`.

Il browser headless rende la stessa UI egui e lo stesso diorama, senza finestre
sul desktop e senza prendere il focus. Il runner usa un profilo effimero: la
classifica dei test resta nel browser del test, non in quella della versione desktop.

```sh
uv pip install --python .venv/bin/python -r scripts/requirements-playtest.txt
.venv/bin/python -m playwright install chromium
scripts/build_web.sh
.venv/bin/python scripts/playtest.py --level Facile --output out/playtest/facile
```

Il comando esegue titolo → nome → livello → tutorial (Esc) → preallerta da una
scheda sulla mappa → Avvia → decisioni → finale → Riprova. Salva PNG e stato
JSON di apertura, piano, crisi, finale e ritorno ai livelli, più `report.json`
con azioni e durata, e `browser.log`. Fallisce se il motore o il browser
segnalano errori, se manca un controllo o se il flusso non arriva al finale.
Il server statico parte su una porta locale libera e si chiude alla fine.
`--url` permette di usare un server già avviato; `--size 1280x720` cambia il
viewport. `--browser-executable <path>` usa un Chromium già installato.

La modalità `?playtest=1` tiene fermo il tempo finché l'agente chiede di
avanzare. `advance` esegue i passi originali di 6 s alla massima velocità CPU,
senza attendere il tempo reale, fino al limite richiesto, alla prima crisi o
alla fine. Cede al rendering e all'input ogni ~40 ms. Non salta fisica, eventi
o decisioni. In una crisi l'agente sceglie sulla UI; `elapse` fa scadere i
conti alla rovescia senza aspettare. Il confronto senza ordini avanza più
passi per frame. Senza `?playtest=1`, i tempi del gioco restano quelli normali.

Per una sessione persistente guidata da un agente:

```sh
.venv/bin/python scripts/playtest.py --interactive --output out/playtest/agente
```

Un oggetto JSON per riga su stdin; una risposta JSON per riga su stdout.
La prima risposta contiene `ready` e lo stato. `state.controls` elenca nomi,
rettangoli e coordinate dei controlli egui (pixel CSS, scala 1). I click
passano realmente dal canvas a winit ed egui. Per controlli con nomi uguali,
`index` sceglie l'occorrenza; si possono anche usare coordinate da screenshot.
`Mappa:<borgo>` apre la scheda sulla mappa; `Borgo:<borgo>` identifica gli
item trascinabili nel piano.

```json
{"op":"state"}
{"op":"screenshot","name":"titolo"}
{"op":"click","label":"Gioca"}
{"op":"type","text":"Giulia"}
{"op":"click","label":"Medio"}
{"op":"key","key":"Escape"}
{"op":"click","label":"Mappa:Castelvento"}
{"op":"click","label":"Preallerta"}
{"op":"click","label":"Avvia"}
{"op":"advance","seconds":10800}
{"op":"screenshot","name":"decisione"}
{"op":"elapse","seconds":46}
{"op":"drag","from":[1260,500],"to":[1280,350]}
{"op":"wheel","y":-300}
{"op":"quit"}
```

`--script <file.json>` esegue un array delle stesse operazioni. `advance` è
accettato solo in esecuzione, fuori da tutorial e conferma evacuazione.
Ogni risposta osserva la fase, il tempo simulato, gli ordini, l'autorizzazione
per borgo e gli errori. Leggere sempre i PNG dopo una prova grafica.

Per la velocità il runner abilita la GPU headless (Metal su macOS).
`--software-rendering` usa SwiftShader sulle macchine senza GPU; è più lento.
Le schermate iniziali e gli scatti richiedono qualche frame di assestamento,
non attese di secondi fisse. Verifica del 2026-10-13 su M4 Pro: flusso completo
con screenshot e Riprova in 37,3 s (Facile) e 37,9 s (Medio), senza errori
browser; test Rust di invarianza di tempi delle crisi ed esito superato anche
con batch accelerati da 40 e 10.000 passi.

## Audio (2026-10-30)

Bevy con `bevy_audio` e la sola decodifica `mp3` (symphonia, Rust puro). Il browser tiene sospeso un AudioContext finché la pagina non riceve un gesto: `index.html` ricorda ogni AudioContext creato (quello di cpal nasce prima) e lo riprende a ogni `pointerdown`, `keydown` o `touchend`. Il jingle del logo, se la pagina non ha ancora ricevuto un gesto, suona solo dopo. **Non provato in un browser.**
