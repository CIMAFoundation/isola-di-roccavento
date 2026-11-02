# Mario (solo build nativa)

**Ricollegato il 2026-10-16.** Si apre con **Ctrl+Maiusc+M** (combinazione nascosta; il tasto Start del pad non lo apre più dal 2026-10-17) nel titolo o nella partita, **in pausa o no** (2026-10-21, richiesta dell'utente; non nel tutorial). Mentre Mario è attivo il mondo parte a **1x** (prima fermo), con la grafica a colori anche se si entra dalla vista tattica, e il pannello permette 0x, 1x, 5x, 10x, 30x, 60x (la pausa del gioco non scade), la UI del gioco è nascosta e la camera della mappa è sospesa. Dei vecchi attori restano solo le squadre (collisione e saluto con F / L3); persone e auto non esistono più. Non provato a schermo (solo test e build).

**Ctrl+Maiusc+M** apre Mario, anche a partita in corso.
Parte davanti all'ingresso della rocca, usando posizione, orientamento e scala
di `island.json`; negli scenari senza rocca usa la strada più vicina alla vista.
Ctrl+Maiusc+M o Esc torna al piano e ripristina camera, viewport e pausa.

| Azione | Tastiera | DualSense |
|---|---|---|
| Movimento | WASD | Stick sinistro |
| Camera | Frecce | Stick destro |
| Zoom camera | Rotella, Q avvicina / E allontana | L1 avvicina / R1 allontana |
| Wing Cap (attiva / disattiva) | V | ○ |
| Salto / nuoto / boost durante il volo | Spazio | × |
| Pugno / calcio | J | □ |
| Accovacciarsi / schianto a terra | Shift | L2 o R2 |
| Scegliere posizione sulla mappa | R | △ |

Il pannello mostra il controller rilevato, energia e ora della simulazione.
La camera può orbitare fino alla vista quasi verticale e variare distanza da
12 a 240 m.
Il Wing Cap resta attivo finché lo disattivi con V / ○; alla morte o al
riavvio Mario riparte senza. Un triplo salto in corsa avvia il volo, guidato
con WASD / stick sinistro. Ogni pressione di Spazio / × durante il volo dà un
impulso di velocità (+40 unità
SM64, massimo 120); a terra resta il salto. Ali e fisica del volo sono gestite da libsm64.

Con R / △ o «Scegli posizione sulla mappa» si apre una vista verticale dell’intera
mappa e Mario resta fermo durante la scelta. Muovi il mouse o WASD / stick sinistro
per spostare il mirino; frecce / stick destro spostano la vista. Rotella, Q/E e L1/R1
regolano lo zoom. Lo zoom (rotella) avvicina verso il puntatore: il punto sotto il mouse resta sotto il mouse.
Il puntatore sceglie anche i modelli: sopra la rocca o la torre (e i tetti) Mario si
posiziona in cima, non a terra (2026-10-20, richiesta dell'utente). Un clic sulla mappa, Spazio / × o «Posiziona» conferma;
Esc, ○, R / △ o «Annulla» torna a Mario senza spostarlo. Puoi scegliere qualsiasi
punto, anche in mare; nei punti occupati si cerca terreno libero entro circa 21 m,
altrimenti la selezione resta aperta. Dopo la conferma si torna alla camera di Mario.
Decisione dell’utente: 2026-10-13.

Il tempo del gioco avanza a **×1**, attraverso `kiosk::step` e `Sim::tick`.
Una crisi o il fine partita chiude Mario e restituisce il controllo al gioco.
L'uscita aggiorna l'anteprima degli ordini, senza applicare il piano proposto
né consumare un'altra pausa. Le interazioni con persone e veicoli sono reazioni
visive temporanee: non modificano agenti, ordini o punteggi.

La collisione usa le mesh effettivamente disegnate: terreno, edifici,
strade, rocca e torre; persone, auto e mezzi operativi aggiornano i collisori
a 30 Hz. Gli alberi sono solo visivi e non bloccano Mario né la camera.
Il mare usa la stessa quota/onda della scena, con fondale sommerso
per consentire il nuoto. Il fuoco attivo e gli edifici ancora in fiamme
provocano danni e la reazione di bruciatura di libsm64. Il modello si ricrea
alla morte o alla conferma del riposizionamento.

Avvio: `cargo run -p game --release`.
La ROM originale USA `.z64` viene letta solo all'apertura di Mario da
`data/mario64/Super Mario 64 (USA).z64`. `MARIO64_ROM` permette un altro percorso.
La ROM viene verificata prima di passarla alla libreria C; non è incorporata
nel programma. Il codice vendorizzato e la sua revisione sono documentati in
`crates/libsm64-native/vendor/README.md`.

La build wasm esclude modulo, libreria nativa, controlli e ROM. Nessun download
è richiesto in compilazione: la sorgente libsm64 è vendorizzata e `cc` la compila
solo per la dipendenza nativa.

Verifiche: `cargo test -p libsm64-native -- --nocapture` (fisica, nuoto, danni,
ownership e cleanup con ROM locale); `cargo test -p game --release mario`
(coordinate, camera e clock ×1/ritorno alla pausa); `cargo check -p game
--target wasm32-unknown-unknown` (nessuna dipendenza libsm64).

Screenshot riproducibile della modalità: `KIOSK_SNAP=out/mario
KIOSK_SNAP_SPEC='name=rocca,phase=pianifica,mario=1' cargo run -p game
--release`. Il campo `mario=1` è solo nativo; il programma salva il PNG e termina.
`mario=<x>:<y>` mette Mario in quel punto (metri, est e nord) invece che accanto
al fuoco; `walk=1` lo fa correre dritto invece di mettersi in posa (con `rec=`,
per filmare il terreno che scorre). Un solo stato con Mario per avvio: il
secondo lo spegne invece di riavviarlo (2026-11-02).

## Passatempi (2026-10-20, richiesta dell'utente)

Solo per divertimento, in `crates/game/src/mario_play.rs`. Leggono dove sta
Mario e cosa fa; non toccano `Game`, ordini, agenti, punteggio o classifica, e
con Mario chiuso non fanno nulla. I progressi durano quanto il programma; solo il
record della corsa è su file (`~/.rocca_ventosa/mario_record.txt`, non scritto se
`KIOSK_SCORES` è impostata). Restano nascosti: niente nel tutorial.

- **Stelle** (colonna gialla visibile da lontano): sul tetto della rocca (solo
  Wing Cap; quota dalla mesh disegnata per scala), una più in alto vicino alla
  rocca (solo Wing Cap), ai piedi della torre (fuori dal suo ingombro), porto,
  una per borgo. Il pannello in alto a destra dà
  l'indizio della più vicina con distanza e direzione. Presa una stella, Mario
  fa la posa della vittoria per 2,4 s e poi torna ai comandi (la danza della
  stella non finisce da sola: `Engine::end_victory`).
- **Monete** lungo le strade (una ogni ≥ 90 m, al massimo 180, mai in mare).
- **Saluti** (F / L3): ogni borgo (entro 90 m dal centro) e ogni squadra hanno
  un fumetto diverso. Prima volta e ripetizioni sono contate.
- **Corsa** dalla colonna azzurra del porto alla colonna azzurra della rocca,
  con cronometro e record. Si annulla se Mario viene spostato (R) o recuperato.
- **Fiammelle**: un pugno vicino a celle in fiamme ne fa uscire uno sbuffo e le
  conta, ma il fuoco vero non cambia (lo dice il pannello).
- **Titoli** (stelle, monete, saluti tutti): compaiono nel pannello.

**Suoni** (2026-10-20): sono quelli della ROM, via `Engine::cue` (`Cue` in
`libsm64-native`). Moneta: tintinnio; stella: suono della stella e fanfara di
raccolta; saluto: «Yahoo!»; fiammella: sbuffo di fiamma spenta; via della corsa:
sparo e «Here we go!»; arrivo: campanella; record: jingle del punteggio alto;
titolo: jingle del rompicapo risolto. Il test della libreria verifica che ognuno
produca audio vero sopra il silenzio. Non udito a orecchio.

Verifiche: `cargo test -p game --release mario` e `cargo test -p libsm64-native`.
Non provato a schermo.
