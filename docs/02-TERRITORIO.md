# 02 — Il territorio

Un solo territorio sintetico e plausibile, riusato con diversi inneschi e venti: l'**Isola di Roccavento**. La scelta finale di territorio e incendi spetta all'utente.

## Com'è fatto

- **Isola di 8×8 km.** La griglia del fuoco è di 20 m (400×400). La riga 0 è il nord, y cresce verso nord, le coordinate sono in metri.
- Il rendering usa un terreno più fine e smussato: i 20 m valgono solo per il fuoco.
- **Dati:** `data/scenarios/rocca_ventosa/`, l'unico scenario del gioco. Contiene `dem.f64`, `fuel.i32`, `cover.u8`, `population.json`, `osm.json`, `game.json` (stazioni, squadre, evacuazione, tutorial, incendi) e il terreno di rendering.
- **Sei borghi:**
  - a est del massiccio: **Castelvento** (190 famiglie, presidio AIB), **Fornaci** (90), **Le Terrazze** (30), **San Rocco** (100);
  - a ovest: **Pian dei Grilli** (90) e **Le Ghiande** (38, un solo nucleo dal 2026-10-21: `tools/factory/ghiande.py`).
- **Aree di raccolta scelte a mano** (2026-10-14, `evacuation.refuges`; agganciate alla rete con `nearest_reachable`; esempio `examples/rifugi.rs`). Prima molti borghi finivano in un'area lontana o oltre il fuoco (Pian dei Grilli a 9 km sopra il valico, Castelvento e San Rocco lungo la SP 12 attraverso l'innesco):

  | Borgo | Area di raccolta | Posizione (m) |
  |---|---|---|
  | Castelvento, Fornaci | il pianoro a nord-ovest (capo della SP 12, terreno aperto) | 3265, 6249 |
  | San Rocco | il porto, a est, lontano dal fuoco | 7900, 3413 |
  | Le Terrazze | l'area di attesa di Castelvento | 5259, 5481 |
  | Pian dei Grilli | l'incrocio della SP 9 con la Costiera, nella piana ovest | 1778, 3582 |
  | Le Ghiande | automatica: l'area di attesa di Pian dei Grilli | 2174, 2536 |

- La **rocca** è in cima, con un anello non combustibile di 130 m.
- Le **aree sicure** dell'evacuazione si ricavano dai dati (`abm::refuge`: nodi stradali con poco combustibile intorno, raggiungibili in auto). Per ogni borgo si sceglie la più vicina su strada, a più di 400 m dalle sue case.
- **Valico** (2026-10-13): una fascia di macchia alta (classe 9, con faville), circa 117 ha, lungo la Strada del Passo. Va da Le Terrazze, passa la sella e arriva alla macchia sopra Pian dei Grilli. Serve perché il fuoco possa arrivare anche ai borghi a ovest: prima nessun incendio colpiva case sia a est sia a ovest. Il fuoco passa a ovest quando il vento gira da nord-est dentro il passo. Larghezza variabile (in media 320 m), bordo frastagliato di macchia media (classe 8) e qualche isola del bosco vecchio, perché nel gioco non sembri una striscia dipinta; il nucleo lungo la strada è sempre continuo.
- **Costa est** (2026-10-13): a nord del porto una baia sabbiosa, un piccolo promontorio e l'angolo nord-est in diagonale, al posto della costa dritta a 300 m dal bordo della finestra; anche il breve tratto dritto a sud del porto è rifatto. Solo terra diventata mare: porto, strade, case e inneschi non cambiano.
- **Strade secondarie** (2026-10-15, richiesta dell'utente: vie alternative che facilitino gli interventi; `tools/factory/strade.py`, scrive in `osm.json` le vie con id ≥ 200): sulla piana est, dove sono gli incendi, la *Variante Nord di Castelvento* (2,0 km, alternativa alla SP 12), la *Strada del Piano* (1,8 km, da Le Terrazze a San Rocco a sud dell'innesco), la *Strada dei Campi* (0,6 km, Fornaci–Le Terrazze) e la *Strada di San Rocco al Mare* (2,2 km, fino alla Costiera). **Incroci** (2026-10-15): dove una via nuova attraversa una strada carrabile lo script mette un vertice comune (`abm::network` unisce le vie solo lì; prima 9 incroci erano «ponti» senza svolta), e una via finisce dove incontra per la prima volta la strada di arrivo (la San Rocco al Mare correva 1,5 km accanto alla Costiera). **Promontorio** (2026-10-15, richiesta dell'utente: il fronte più veloce va attaccabile entro la prima ora): la *Strada della Sella* (2,2 km, 13 %, Costiera–San Rocco sulla sella a 246 m), la *Strada dei Tornanti* (1,5 km, 16 %, dalla Sella alla piana a sud dell'innesco del `medium`) e la *Strada di Mezzacosta* (1,8 km, 13 %, dalla Strada del Piano ai tornanti alti della Strada del Passo, sulla linea della testa del `medium` a 30–90 min). Instradate con tornanti (16 direzioni di passo, solo queste tre). A ovest di x 5800 la cresta non si passa sotto il 20 % e i 5 km (versante sud al 40–50 %). **Non sono tagliafuoco:** `fuel.i32` non è toccato, il fuoco le attraversa; il pericolo sulle strade viene da `fire::threat`. Collegamenti attraverso il massiccio (Le Ghiande, Pian dei Grilli) non esistono con pendenze da camion (provato fino al 20 %). Con la SP 12 aperta i tempi di arrivo delle squadre e di evacuazione non cambiano (Castelvento 37 min, San Rocco 23, Fornaci 22, Le Terrazze 12); da provare con la SP 12 chiusa dal fuoco.
- **Umidità del combustibile:** 5 % in tutti gli incendi (`moisture_pct` del caso, sovrascrivibile; il 7 % di gioco è stato ripristinato a 5 % dall'utente, 2026-10-15). Uniforme. La leva vera è il vento.
- **Squadre** (2026-10-14): 3 squadre AIB al presidio di Castelvento (ex caserma, 2026-10-16: tutte le squadre sono AIB), 2 squadre al presidio di Pian dei Grilli (dal 2026-10-14, dall'altra parte dell'isola rispetto a Castelvento). Ogni squadra è un'autobotte con 3 persone.
- **Esagoni** di 500 m: valgono solo quelli attraversati da una strada carrabile della rete della caserma (`rocca::hex`).

## Incendi (`game.json`)

Il vento è indicato con la direzione **da cui** soffia. Gli inneschi sono in metri.

| Uso | Caso | Innesco | Vento | Durata | Note |
|---|---|---|---|---|---|
| Tutorial | `tutorial` | (3792, 3295), sotto Le Ghiande | da 135°, 20 km/h | 150 min | Seme 1. Senza ordini: 90 ha, 17 case colpite, 45 persone colte. Con il copione: 6 ha, nessuna persona colta. Trovato con `examples/tutorial_cerca.rs`. |
| Partita | `medium` | (5590, 4370) | da 119°, 36 km/h, umidità 5 %; gira a 50° (36 km/h) al minuto 45 | 300 min, segnalato al minuto 30 (`start_min`) | Seme 1. Senza ordini (ha, case, persone): seme 1 512, 28, 172; seme 2 361, 38, 185; seme 3 506, 35, 190 (35 km/h: 467, 28, 137 / 384, 33, 195 / 386, 33, 257). |
| (riserva) | `easy`, `expert` | come prima | | 180 min | Non usati dal gioco. |

## Ricerca di nuovi scenari (2026-10-16)

Priorità dell'utente: **varietà degli impatti**, oltre a una sfida giocabile.
Cambiare vento e innesco senza cambiare i borghi colpiti non basta. La suite
`tools/factory/scenarios.py` misura per borgo persone, case e tempi del primo impatto
senza ordini; confronta quattro politiche sui semi 1–3, conferma sui semi 4–5 e
prova primi ordini ritardati di 10 min. Il gioco continua a girare: solo prove
headless, nessuna finestra o intervento sul focus.

65 combinazioni esplorative, poi 9 prove mirate sui nuclei sparsi, per 557 run
salvati (212 nella verifica mirata finale). 17 test della suite superati. La
politica `planner=nuclei` considera case visibili, fiamme attuali e cambio di vento
annunciato, assegna le squadre per ETA e conserva gli incarichi adatti. Massimo
15 spostamenti e 4 evacuazioni. **Non è un automatismo aggiunto al gioco**.

Tre configurazioni native approvate dall’utente e aggiunte al selettore operatore, tutte con segnalazione al minuto 30:

| Profilo | Identificativo | Innesco; vento; umidità; durata | Impatti senza ordini nei semi 1–5 |
|---|---|---|---|
| Attraversamento del valico | `impatti_valico_due_lati_30_6` | (5590, 4370); 30 km/h da 119°, poi 50° al min 75; 6%; 240 min | Le Terrazze e Le Ghiande; Castelvento marginale in un seme. 2–3 borghi totali. |
| Pressione su quattro borghi | `impatti_nuclei_valico_costante_lungo_28_7_0_60` | (5590, 4370); 28 km/h da 119°, costante; 7%; 300 min | Castelvento, Fornaci, Le Terrazze e Le Ghiande: quattro impatti consistenti in tutti i cinque semi. |
| Rotazione da est | `impatti_nuclei_est_giro_lungo_28_6_75_60` | (5870, 4750); 28 km/h da 119°, poi 50° al min 75; 6%; 300 min | Le Terrazze e Le Ghiande; Castelvento consistente in 4/5 semi; San Rocco marginale in 4/5. 3–4 borghi totali. |

Con il piano ordinario, nei cinque semi: nessuna unità persa; persone colte
0/0/0/0/0 per valico e quattro borghi, 0/0/7/0/0 per rotazione est. Case colpite:
0/0/0/1/0 al valico, zero negli altri due casi. Il valico supera anche i filtri del
test di ritardo (fino a 17 case colpite ma nessuna persona o unità persa).
Gli altri due casi sono **meno indulgenti**: con 10 min di ritardo perdono un'unità
in un seme; nella rotazione est il ritardo porta anche a 44 persone colte in un
altro seme. Queste fragilità restano esplicite, non vengono promosse come robustezza.

Rapporto in `results/scenario-impacts/focused/report.md`; i tre casi, configurazioni
complete e cinque piani per caso sono in `variety-cases.json`, `review-configs/` e
`review-plans/`. `game-cases.json` mantiene la sola selezione che passa anche i
filtri rigorosi del ritardo. Su richiesta dell’utente i tre casi sono ora in
`game.json`, esposti tramite `levels` nel solo menu operatore. `game: medium`
rimane il default; tutorial invariato. La demo in esecuzione non è stata
riavviata né portata in primo piano. La prova umana resta utile per il bilanciamento. Istruzioni e criteri in
[`tools/factory/SCENARI.md`](../tools/factory/SCENARI.md). Le vecchie ricerche della
sola giocabilità restano separate in `results/scenario-playable/`.

## Come si modifica

- La vecchia pipeline di generazione è stata **cancellata** (2026-10-13, resta nella storia git): il territorio non si rigenera da zero.
- Si modificano **direttamente i file pubblicati** (`fuel.i32`, `game.json`, `population.json`, `osm.json`), con uno script ripetibile quando serve. Per esempio `tools/factory/valico.py` ridisegna il valico in `fuel.i32` partendo dal combustibile precedente (dalla storia git) e dà sempre lo stesso file; `tools/factory/costa_est.py` ha tagliato la costa est (`render_terrain.f32`, `dem.f64`, `fuel.i32`, `cover.u8`) e, rieseguito, non trova più nulla da tagliare. Entrambi sono autonomi (numpy, scipy).
- **Per provare una variante:** creare `data/scenarios/<id>/` con link simbolici a `rocca_ventosa` più i soli file cambiati, poi selezionarla con `ROCCA_SCENARIO=<id>` (`scenario::id()`). Vale per la CLI, i test e il gioco. Niente copie dei dati (richiesta dell'utente).
- La vegetazione disegnata nel gioco si legge da `fuel.i32` (`vegetation.rs`), non da `cover.u8`.
- **Specie disegnate per classe di combustibile** (2026-10-31, richiesta dell'utente; solo aspetto, il combustibile non cambia): conifere 10–12 pino domestico, pino d'Aleppo/marittimo e un 10 % di cipressi (10 e 12 più pino domestico, 11 più Aleppo); latifoglie 4–6 leccio, sughera, carrubo (in 6 anche ulivo); macchia 7 gariga (cisto, rosmarino, erica, ginestra), 8 macchia (lentisco, mirto, cisto, ginestra, erica), 9 macchia alta (lentisco, corbezzolo, erica, mirto); nei prati ulivi isolati. Le specie vengono a macchie di qualche decina di metri (`vegetation::species_for`). Le specie di un gruppo hanno la sagoma del gruppo, così ciò che si vede è ciò che brucia.
- Dopo ogni modifica del territorio si **rifanno le prove** di `03-VERIFICHE.md`.

## Cosa rende buono il territorio

- Gli incendi colpiscono davvero i borghi, anche quelli a ovest, e lasciano **finestre d'intervento** utili. Non si ottimizza per il numero di case colpite.
- Esistono **compromessi**: due borghi minacciati insieme, mezzi che non bastano, un cambio di vento che ribalta il piano.
- Strade e aree sicure esistono e sono raggiungibili. I percorsi delle squadre hanno ETA plausibili.
- Nessuna strategia semplice vince sempre: si verifica con le prove di `03-VERIFICHE.md`.

## Aperto

- Segnalazione in ritardo (2026-10-14, `start_min`). Strategia semplice: «tutte le squadre sull'innesco, evacua Le Terrazze», all'inizio. Piano ragionato: quattro pause; squadre a Le Terrazze, Castelvento e Le Ghiande; evacuazioni a turno di Le Terrazze, Le Ghiande, Pian dei Grilli e Fornaci.

  | Ritardo | Seme | Strategia semplice: ha / case / persone colte / stelle P-C-E | Piano ragionato: stelle P-C-E |
  |---|---|---|---|
  | 0 | 1 / 2 | 2 / 0 / 0 / 5-5-5 · 3 / 0 / 0 / 5-5-5 | — |
  | 20 | 1 / 2 | 497 / 118 / 333 / 2-2-2 · 11 / 0 / 0 / 5-5-5 | — |
  | **30** | 1 / 2 | 481 / 127 / 343 / 1-1-2 · 469 / 126 / 333 / 2-1-2 | 5-2-3 · 5-2-2 |
  | 40 | 1 / 2 | 483 / 124 / 355 / 1-1-2 · 637 / 135 / 333 / 2-1-1 | — |

  Scelto 30. Da verificare con il seme 3, con «evacua tutto» e con piani migliori: oggi le Case restano basse anche col piano ragionato.
