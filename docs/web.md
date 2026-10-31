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

Il test touch ha ora una lingua esplicita (`it-IT`), coerente con i testi che cerca. La prova completa preesistente si interrompe anche sulla build iniziale nella parte di trascinamento/cancellazione del tutorial: non viene considerata superata. La misura della partita completa usa il runner di memoria separato. Avvio, nome, tutorial e rotazione verificati con `--smoke`; nessuna prova su telefono fisico.

## Pagina introduttiva (2026-10-18)

`web/index.html` mostra prima una pagina HTML+CSS nello stile di cimafoundation.org (hero con una scena di incendio e un riquadro «Dal gioco», tre card, un paragrafo sulla ricerca, chiusura, crediti). Testi brevi e narrativi, senza meccaniche, durata o numeri (2026-10-19, richiesta dell'utente). `game.js` e il `.wasm` si scaricano solo quando si preme «Gioca» (`import()` dinamico in `start()`); con `?playtest=1` si salta l'intro. Dopo «Gioca» compare la schermata del titolo del gioco, con il suo «Gioca» (decisione dell'utente, 2026-10-18: non va saltata). Immagini in `web/img/`, copiate da `build_web.sh`: screenshot veri del gioco (rifatti il 2026-10-19 con i nuovi effetti grafici; `KIOSK_SNAP`, 1600×1000): `tattica-*` con la UI tattica (ritagliati senza banner e chat, `t=45/90/120`, seme del menu), `scena-*` (`borgo`, `cicatrice`) e `hero` senza interfaccia (`ui=0,marks=0,iso=0`, `KIOSK_GFX=-dof KIOSK_TACTICAL=0 KIOSK_BOOST=0`, `t=80/140/165/170`). Provata solo come pagina statica (desktop e 390 px), non con la build wasm.

## Dati separati dal codice e profilo grafico (2026-10-19)

**Wasm più piccolo (2026-10-30, iPhone che si ricarica):** modelli (`assets/models/*.json`, 10,7 MB) e immagini (personaggi, icone, logo) non sono più compilati nel `.wasm` per il web: viaggiano con i dati (`build_web.sh`, `datafs::take`, che libera anche la copia della pagina dopo la lettura); il desktop li tiene compilati dentro. Profilo `wasm-release` con `opt-level = "z"` e LTO completo; `wasm-opt -Oz` nella CI (binaryen version_133, checksum verificato). `.htaccess` con compressione (`mod_deflate`). Misure locali senza `wasm-opt`: `.wasm` da 49 a 29 MB (codice 31,3 → 22,5 MB, dati 19,3 → 6,7 MB), compresso da 12,5 a 8,6 MB. WebKit headless (`playtest_mobile.py --browser webkit --smoke --reloads 2`): parte, memoria lineare 258 MiB al titolo e 275 MiB nel tutorial (misura non confrontabile con quelle del 2026-10-21, fatte su un'altra build). **Non provato su iPhone.**

**Server lento (2026-10-30):** games.cimafoundation.org serve circa 500 KB/s, non comprime (l'`.htaccess` con `mod_deflate` è ignorato) e in Chromium headless il download HTTP/3 del wasm da 25 MB si è interrotto. `build_web.sh` scrive anche `game_bg.wasm.gz` (8,6 MB): la pagina lo scarica e lo decomprime in streaming con `DecompressionStream` (il `.wasm` pieno resta per i browser senza), e riprova una volta se il download si interrompe. Smoke test WebKit e Chromium passati in locale. Ogni download (dati, `game.js`, wasm) si riprova fino a 3 volte («TypeError: Load failed» su iPhone). Il proxy del server (FireShield, `x-vc-cache`) teneva in cache `index.html` e `data/manifest.json` per circa 15 minuti: dopo un deploy un iPhone emulato (WebKit) ha avuto il gioco nuovo con l'elenco dati vecchio («assets/models/town.json is not in the page's data»). Ora l'URL del manifest ha l'hash e l'`.htaccess` chiede `Cache-Control: no-cache` per HTML e JSON. **Aperto:** velocità e stabilità del server (Ergonet), da segnalare o da spostare su una CDN.

**Memoria iniziale da 320 MiB (2026-10-31, iPhone 13 con Chrome: la pagina muore appena dopo il logo, anche con i download riusciti).** Il wasm partiva con circa 1 MiB di memoria lineare e la allargava a piccoli passi fino a 260–280 MiB mentre costruiva la scena. Su iOS WebKit può dover ricopiare tutta la memoria a ogni allargamento, e nel picco ne servirebbe il doppio. Ora il linker la riserva subito (`.cargo/config.toml`, `--initial-memory`): non serve allargarla durante l'avvio. Le pagine non toccate non occupano RAM. Pubblicata e verificata sul sito (il wasm scaricato dichiara 320 MiB): **sull'iPhone muore ancora**.

**Registro dell'avvio (2026-10-31).** La pagina scrive in `localStorage` (`rocca_avvio`) i passi dell'avvio (`play` con schermo e profilo, `wasm`, `data`, `init`), errori, `webglcontextlost` e, a gioco avviato, una riga al secondo con i frame disegnati e la memoria wasm (le ultime sei). Uscendo dalla pagina il registro si cancella. Se alla riapertura c'è ancora, la pagina era morta: lo mostra in cima alla pagina introduttiva, con lo user agent, da fotografare. Primo esito dall'iPhone 13 (iOS 27.0.1, Chrome 156): `play`, `wasm`, `data`, `init`, poi `run 2 320 MiB` e la morte: due frame, memoria wasm ferma.

**Simulatore iOS 18.4 (Safari, Mac), `footprint` dei processi WebKit (2026-10-31):** a regime la pagina (WebContent) occupa 535 MB e la GPU 150 MB; **all'avvio la pagina sale a 1,9–2,1 GB per 1–2 s**, prima che la GPU salga. La sola compilazione del wasm, senza gioco (`prova.html`), fa lo stesso picco (1,8 GB, poi 147 MB). Il codice sono 102 mila funzioni per 22 MB, nessuna sopra 232 kB. Il simulatore non ha i limiti di memoria né i thread dell'iPhone: indizio, non prova.

**Interruttori dall'indirizzo (2026-10-31), per isolare la causa sul telefono:** `?audio=0` (niente suoni del gioco), `?orizzonte=0`, `?raggi=0`, `?gfx=` come `KIOSK_GFX` (es. `veg=0,terrain_stride=8`), `?trasmissione=0` (piante senza luce attraverso le foglie), `?stato=0` (piante senza lo stato del fuoco; dal 2026-10-31 le piante lo ricavano nel vertex shader da una texture RGBA8 sulla griglia del fuoco, un texel per cella, e sono istanze di una mesh per varietà, 33 entità in tutto), `?trama=0` (piante senza l'atlante delle foglie: solo il colore del vertice; dal 2026-10-31 la trama procedurale non c'è più). Esiti sull'iPhone 13: `prova.html` regge (la compilazione da sola non uccide la pagina); l'isola spoglia parte; il colpevole è la **vegetazione** (indicazione dell'utente). Con il profilo del telefono sono 4197 piante, 0,11 M triangoli (case 0,14 M, terreno 0,35 M): non è la quantità di poligoni. Nessuno dei tre interruttori dello shader basta; con un quinto delle piante (`?gfx=veg=0.005`) l'isola si vede e la pagina muore poco dopo, a 2 frame al secondo. **Causa probabile:** quando la prima pianta di un blocco prende fuoco (l'incendio della demo arde già sul titolo) si costruisce lo strato dei resti bruciati, e con le poche piante del web era una **mesh vuota** (0 vertici, 0 indici, visto nella console WebKit). Il Mac la tollera. Ora una mesh dei resti senza resti riceve un triangolo di area nulla (`vegetation::build_char_layer`). `?brucia=0`: le piante non bruciano a vista (interruttore di prova). `?fuoco=0`: le mesh del fuoco (fiamme, fumo, scintille, riscritte a ogni frame) non si aggiornano. Il registro dell'avvio salva anche gli ultimi 30 messaggi della console (log del gioco, errori WebGL e wgpu): in viaggio, senza cavo né Web Inspector, è l'unica finestra sul telefono. Esiti: con `?fuoco=0` e con `?audio=0` muore lo stesso; la console finisce con la scena costruita (13,4 s) e nessun errore, poi la pagina muore in silenzio (processo ucciso da iOS, probabilmente per memoria). `?materiale=standard` (piante con lo `StandardMaterial` di Bevy) è tolto il 2026-10-31: le piante ora sono istanze disegnate dal loro materiale. **Da confermare sull'iPhone.** **`prova.html`** scarica e compila il wasm come la pagina e non disegna nulla: se muore anche lei, la colpa è della compilazione.

**LOD delle piante senza riscritture di mesh (2026-10-31).** Prima ogni blocco di piante passava tra i due livelli delle proxy (oltre `FAR_TIER_MIN_M` = 2000 m sul web) **scambiando la lista degli indici della sua mesh**: Bevy la riestraeva e la ricaricava intera sulla GPU. Con la camera del titolo che deriva, in WebKit (iPhone emulato, profilo telefono) erano **136 cambi di livello in 40 s**, ognuno una ricarica. Ora il livello lontano è una **terza mesh statica** per blocco (solo i vertici che usa, 0,05 M contro 0,10 M delle proxy) e il LOD cambia solo la `Visibility` delle entità (`vegetation::lod`). Le mesh delle proxy restano solo sulla GPU (`RENDER_WORLD`, indici a 16 bit): sparisce la copia CPU nella memoria wasm (circa 6 MB col profilo telefono), la GPU ne aggiunge circa 2,5 MB. Verificato con un log temporaneo degli `AssetEvent::<Mesh>::Modified` in 30–40 s di titolo: nessuna mesh delle piante modificata. **Restano riscritte a ogni frame tre mesh di `fire_view`** (fiamme, fumo, scintille: `meshes.insert`), più una manciata una sola volta. Scartati: `VisibilityRange` di Bevy (distanza dal centro del blocco e non dal punto più vicino, senza isteresi; la dissolvenza disegna due livelli insieme e vuole il dithering nello shader della vegetazione; su WebGL2 un buffer uniforme di 64 intervalli) e un LOD nel vertex shader (i vertici di dettaglio si elaborano comunque, servono vertici duplicati dove i livelli si toccano). **Da provare sull'iPhone.**

**Dati.** I file del territorio (`data/scenarios/rocca_ventosa`, `data/behaviours`, `fuels_eu12.json`; dal 2026-10-30 anche `data/sounds`, 0,65 MB di MP3) non sono più nel `.wasm`. `build_web.sh` li comprime in gzip in `target/web/data/` e scrive `data/manifest.json` (percorso, URL con hash del contenuto, byte). La pagina scarica in parallelo il `.wasm` (compilato in streaming) e i file, li mette in `window.roccaData` e solo allora avvia il gioco; `datafs` li legge di lì (`crates/datafs`). Se il browser ha `DecompressionStream` (Chrome 80, Safari 16.4, Firefox 113) decomprime lui; altrimenti i byte restano gzip (`window.roccaDataGzip`) e li decomprime il wasm (`flate2`, a ogni lettura). Ogni file sta in Cache Storage (`rocca-data`) con l'URL con hash: una nuova build del codice non fa riscaricare i dati, e viceversa. Dati: 27 file, 4,5 MB gzip (15,9 MB grezzi). Con la decompressione nel browser restano in memoria JS circa 16 MB; con quella nel wasm solo i 4,5 MB compressi. Le due strade non sono state provate entrambe su un browser vecchio.

**Profilo.** La pagina sceglie `window.roccaProfile` e `quality::Graphics::from_env` lo legge: **`bassa`** (`Graphics::WEB`, il profilo di sotto) su telefoni e tablet (puntatore primario grossolano, `(pointer: coarse)`, o `userAgentData.mobile`); **`media`** (`Graphics::WEB_MEDIA`) su computer: come `WEB` più ombre del sole (2 cascate, 1024), terreno a passo 2 e vegetazione 0,05. `?q=bassa` o `?q=media` forzano il profilo, per le prove. I tetti del fuoco, il mare e la risoluzione del suolo restano uguali nei due profili. Dal 2026-10-21 su computer il livello è **adattivo** (`adaptive.rs`, vedi `TODO.md`): parte da `WEB_TOP` (`WEB_MEDIA` con ombre da 3 cascate, 2048) e scende se i frame sono lenti; `?q=` lo spegne. Verificato in Chrome su desktop con entrambi i valori di `?q=` (parte, grafica a vista); **non misurati gli FPS** (dal 2026-10-20 il terreno web usa colore e normale per vertice, senza le texture `ground_*`: WebGL2 ha 16 unità texture, vedi `TODO.md`) e non provato su un telefono vero.

## Lingua (2026-10-21)

Il gioco segue la prima lingua supportata tra quelle preferite del browser (`navigator.languages`, anche per prefisso: `de-AT` → `de`; altrimenti inglese, 2026-10-30), dopo l'ultima scelta salvata in `localStorage`; `?lang=de` la forza (la pagina la passa al wasm come `window.roccaLang`). I cataloghi `data/i18n/*.json` sono nel manifesto dei dati come gli altri (`scripts/build_web.sh`; `keys.json` è escluso). Il pulsante delle bandiere del titolo c'è anche sul web.

**Pagina introduttiva tradotta (2026-10-30, richiesta dell'utente):** stessa regola del gioco: `?lang=` > scelta salvata (`rocca_ventosa_lingua`, condivisa con il gioco) > lingua del browser > inglese. Un selettore 🌐 nell'intestazione cambia lingua alla pagina e al gioco. Testi (anche caricamento, errori, nome, toast della ROM) in `tools/i18n/landing.py`, che genera `web/i18n.js` (`python3 tools/i18n/landing.py` dopo ogni modifica); «Gioca», «Come ti chiami?», «Continua» vengono dai cataloghi del gioco. La pagina passa sempre la lingua al gioco in `window.roccaLang`. Con `?lang=tlh` la pagina resta in inglese e il gioco va in klingon. Traduzioni automatiche, non riviste. **Non vista nel browser**: provata la logica con node (de-AT→de, ja→en, pt-BR→pt, scelta salvata, tlh).

## Telefono e budget grafico (2026-10-15)

La versione web usa un profilo leggero anche sui computer: terreno a 20 m,
vegetazione al 2,5% con sagome semplificate (le mesh dettagliate e le chiome
nascoste non vengono allocate), mare a 60 m vicino all'isola e 600 m al largo.
Niente ombre, HDR, profondità di campo, bloom o MSAA. Il canvas disegna un pixel
per pixel CSS anche sui display Retina; gli input sono normalizzati alla stessa
scala. Le mesh statiche di terreno e mare rilasciano la copia CPU
dopo il caricamento sulla GPU, anche nella versione locale, a geometria e resa
invariate. I ritratti conservano invece la copia CPU: bevy_egui richiede che
l'immagine resti negli asset per mantenere la texture registrata.
Il fuoco conserva al massimo 600 celle con fiamme,
100 particelle di fumo e 80 braci, con lo strato a terra a 20 m.
Sono semplificazioni visive: dati del territorio, simulazione, ordini e punteggio
restano quelli della versione desktop. Il menu web non permette di riattivare effetti pesanti.

L'interfaccia compatta si attiva sotto 1000 px di larghezza o 600 px di altezza:

- vista iniziale dall'alto (nadirale), nord in alto e zoom ampio sull'incendio;
- verticale: istruzioni in basso; orizzontale: tutorial a destra, mappa a sinistra;
- la schermata del titolo è quella del desktop (logo, titolo, sottotitolo, «Gioca»), in scala sulla larghezza;
- apertura, scelta dello scenario, finale e menu scorrono se non entrano nello schermo;
- il nome usa un campo HTML con tastiera del telefono;
- un dito sposta la mappa, due dita fanno zoom, rotazione e spostamento;
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

Pubblicazione: `.github/workflows/pages.yml` costruisce e pubblica su GitHub Pages a ogni push su `main`, `settimana-protezione-civile` o `nuovo-gioco`. Indirizzo: https://cimafoundation.github.io/isola-di-roccavento/ (repository `CIMAFoundation/isola-di-roccavento` dal 2026-10-30; in futuro games.cimafoundation.org/isola-di-roccavento)

**games.cimafoundation.org/isola-di-roccavento** (2026-10-30): hosting CIMA su Ergonet (`web-lnx345.ergonet.host`, 149.3.144.47); la radice FTP dell'utente `cima-games` è la radice di games.cimafoundation.org (c'era la pagina segnaposto di Ergonet). Da `main`, `pages.yml` copia `target/web` con `lftp mirror --reverse --delete` in FTPS (porta 21, certificato `*.ergonet.host` verificato: per questo l'host è `web-lnx345.ergonet.host` e non l'IP) nella cartella `isola-di-roccavento` (variabile `DEPLOY_DIR` del repository per cambiarla). Secret: `DEPLOY_HOST`, `DEPLOY_USER`, `DEPLOY_PASSWORD`. L'SFTP sulla porta 2201 accetta solo chiavi e non è usato. Il passo è `continue-on-error`: se fallisce, GitHub Pages esce lo stesso. Il server mandava `game_bg.wasm` senza tipo MIME e il gioco non partiva (2026-10-30): `build_web.sh` scrive un `.htaccess` con `AddType application/wasm .wasm` e `index.html` compila il file intero quando il tipo manca. **Aperto:** certificato HTTPS per games.cimafoundation.org (oggi copre solo `*.ergonet.host`).

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
