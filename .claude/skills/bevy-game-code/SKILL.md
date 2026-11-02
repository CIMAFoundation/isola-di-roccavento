---
name: bevy-game-code
description: Scrivere e mantenere il codice di gioco Bevy (ECS, schedule, stati, messaggi, osservatori) di un serious game di protezione civile, e misurarne e migliorarne le prestazioni (CPU contro GPU, budget per frame, draw call, istanze, allocazioni). Usare quando si aggiunge o si sposta un sistema, si aggiorna la versione di Bevy, compare un panico B0001/B0002, un effetto dipende dagli FPS, o il gioco «va a scatti».
---

# Codice Bevy e prestazioni

Il gioco Bevy è il guscio grafico attorno al nucleo di simulazione (`hazard-sim-integration`):
legge lo stato e lo disegna. Questa skill riguarda come scrivere quel guscio senza bug di
ordine, di versione o di frame rate. Il rendering vero (shader, materiali, vegetazione) è in
`bevy-diorama-rendering`.

## 1. Versione

- **Prima di scrivere codice si legge la versione** in `Cargo.toml` e `Cargo.lock`. Non si
  mescolano frammenti di versioni diverse, e non si passa a una minor nuova di nascosto:
  l'aggiornamento è un lavoro a sé, con la sua guida di migrazione.
- Cambi recenti che fanno perdere tempo (fino alla 0.20):
  - niente più bundle: si generano i componenti, e i *required components* completano il
    resto;
  - `Time::delta_secs()`, non `delta_seconds()`;
  - gli eventi bufferizzati sono **messaggi**: `#[derive(Message)]`, `MessageReader`,
    `MessageWriter`, `add_message`;
  - dalla 0.19 una `Resource` vive su un'entità propria: si deriva solo `Resource`, e una
    `Query<Entity>` o `EntityRef` senza filtri può prendere anche le risorse (panico B0001
    o B0002 accanto a un `Res<T>`). Si aggiunge un componente marcatore o `Without<IsResource>`;
  - osservatori del ciclo di vita nella forma `On<Add<T>>` (non `On<Add, T>`); il picking
    usa `PointerClick` e `PointerPress`;
  - `NextState::set` rifà sempre la transizione, anche verso lo stesso stato: se non lo si
    vuole, `set_if_different`;
  - `Name::new` vuole una stringa `'static` o una `String` propria.

## 2. Sistemi e ordine

- **I sistemi girano in parallelo per default.** Un ordine che «funziona per caso» si
  rompe dopo un refactor. Si ordinano solo i sistemi che dipendono davvero l'uno
  dall'altro (`.after()`, `.chain()`, set di sistemi), e il resto si lascia libero.
  Con la feature `debug` lo `shuffle_seed` della schedule mescola l'ordine e fa emergere
  le dipendenze nascoste.
- I comandi (`Commands`) si applicano a un punto di sincronizzazione successivo:
  un'entità generata non compare nella query dello stesso sistema.
- Due query mutabili in conflitto: `With`/`Without` per separarle, oppure `ParamSet`.
- `run_if` per accendere i sistemi per fase (intro, tutorial, partita, finale). Un sistema
  spento è il modo più semplice di avere un meccanismo **inerte quando è spento**.
- Un plugin per area (suono, previsione, segni, Mario), con risorse e sistemi propri.

## 3. Tempo e frame rate

- Tutto ciò che si muove nella grafica si scala con `delta_secs()`. Per gli inseguimenti
  (camera, icone che si spostano, quota filtrata) si usa lo **smorzamento esponenziale**
  `a = 1 - exp(-k·dt)`, non un `lerp` con un fattore fisso, che a 30 FPS sembra molle e a
  120 FPS rigido.
- Su un salto voluto (clic su un avviso fuori schermo, riavvio, cambio di fase) si azzera
  lo smorzamento o si usa una transizione breve: niente camera che attraversa l'isola.
- La **simulazione** non segue il frame: avanza a passi di tempo simulato
  (`hazard-sim-integration` §2). La grafica non integra mai nulla per frame.

## 4. Prestazioni: il metodo

1. **Uno stato riproducibile** del caso peggiore (lo stesso harness degli screenshot:
   `game-snapshot-harness`), build `--release`, sulla macchina di riferimento. Le misure
   in debug non contano.
2. **Prima la divisione CPU/GPU**: tempo del frame, thread principale, thread di
   rendering, GPU. Il più lento dei due fissa gli FPS; si lavora solo su quello.
   - GPU: trasparenze sovrapposte (fumo, particelle), ombre, risoluzione (un fattore di
     pixel 2 sono quattro volte i pixel), shader pesanti.
   - CPU: draw call e cambi di materiale, ricalcoli a ogni frame, query su tutta la scena,
     percorsi, allocazioni.
3. **Si toglie il costo più grande**, preferendo i rimedi di algoritmo (fare meno, più di
   rado, in cache, con una griglia spaziale) alle micro ottimizzazioni.
4. **Si rimisura** nello stesso stato. Se non migliora, si torna indietro. L'immagine prima
   e dopo deve restare la stessa.
5. **Budget scritti**: millisecondi per sottosistema e tetti per gli asset, controllati
   dalle prove, altrimenti le regressioni passano in silenzio. Un esempio a 60 FPS
   (16,7 ms): simulazione 3, logica e UI 3, invio del rendering 4, margine il resto.

Rimedi tipici in Bevy:

- **Istanze e materiali condivisi** per ciò che si ripete (alberi, case, fiamme): un
  materiale diverso per ogni oggetto spezza il batching.
- **Pool** per ciò che nasce e muore spesso (particelle, icone, getti d'acqua): si
  nasconde e si riusa invece di generare e distruggere a ogni frame.
- **Zero allocazioni nel frame stabile**: `Vec` riusati in `Local`, stringhe della UI
  ricostruite solo quando il valore cambia (`Changed<T>`, o un confronto col valore
  precedente).
- Geometria statica: copia CPU rilasciata dopo il caricamento (`RenderAssetUsages`).
- **Sui telefoni** le prestazioni calano dopo qualche minuto per il calore: si lascia
  margine, si limita il frame rate e si misura dopo 5 minuti, non al primo secondo.

## 5. Esempio: Isola di Roccavento

Bevy 0.20 (`crates/game/Cargo.toml`), egui per l'interfaccia. Messaggi già migrati
(`SimRestarted`, `MessageReader`). Smorzamento esponenziale della camera in
`crates/game/src/kiosk/view.rs`. Misure per stato con `KIOSK_SNAP_BENCH`
(`crates/game/src/bench.rs`, istruzioni in `CLAUDE.md`), livelli in `quality.rs`,
memoria in `memory.rs`.

---

Fonte: adattato e riassunto da `bevy-ecs` e `performance-optimization` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0). Nessun testo copiato; le parti su
Roccavento sono del progetto.
