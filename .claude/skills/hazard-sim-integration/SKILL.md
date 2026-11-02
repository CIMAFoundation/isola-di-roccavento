---
name: hazard-sim-integration
description: Integrare un modello fisico di rischio naturale (automa cellulare di incendio come PROPAGATOR, modello idraulico, frane, dispersione) dentro un gioco in tempo reale. Usare quando si progetta l'architettura simulazione/gioco/rendering, si aggiunge una meccanica che modifica la fisica (spegnimento, argini, evacuazione), si scelgono la compressione del tempo o la previsione per il giocatore, o si cerca un bug di non riproducibilità.
---

# Un modello fisico dentro un gioco

## 1. Architettura: una sola autorità

- **Un solo oggetto possiede simulazione e regole** (es. `Game`): pausa, ordini, passo,
  esiti, punteggio. Lo usano allo stesso modo il runner headless, i test, il laboratorio
  e la grafica.
- **La grafica legge e basta.** Fisica, punteggio e decisioni non vanno nel rendering.
  Se il rendering deve sapere qualcosa (strade chiuse, pericolo di una squadra, tempo
  residuo), il nucleo espone una funzione e la grafica la chiama.
- **Ciò che il giocatore vede e ciò che il finale conta vengono dalla stessa regola.** Per
  esempio il timer dell'evacuazione e il tempo annunciato vengono da `remaining_min`, e le
  «persone raggiunte» sullo schermo e nelle stelle da `Family::state`.
- **Anteprime senza effetti:** `plan_move` e `plan_evacuation` calcolano percorso e tempo
  senza cambiare lo stato. Va scritto un test che lo verifica.
- Lo stato del rendering che si ricorda qualcosa (oggetti bruciati, particelle, chiavi dei
  segni) va ripulito al riavvio. Tutto il resto si ricalcola dal nucleo a ogni frame.

## 2. Il tempo

- **Tutto ciò che si accumula si integra sul tempo simulato**, mai sul numero di update o
  di frame (calore, acqua, preparazione, code). Va scritto un test di invarianza: stessa
  partita a velocità e passi diversi, stesso esito, anche con passi più piccoli del ritmo
  interno del modello.
- **Compressione:** dalla durata desiderata della partita si ricava il fattore. Per
  esempio 5 h simulate in circa 2,5 min reali danno ×120. Pochi stati: in corsa o fermo.
  La pausa ferma il mondo, non lo rallenta.
- **Il ritmo interno del modello** (passi del CA, ricalcolo dei percorsi ogni 60 s) è
  indipendente dalla velocità di gioco.
- **Riproducibilità:** stessi seme, stato, ordini e tempi simulati danno lo stesso
  risultato. Il seme si sceglie (menu operatore) e c'è un seme predefinito.

## 3. Accoppiamento agenti → fisica

- Gli interventi passano **dalle condizioni al contorno del modello**, non da scorciatoie:
  - l'acqua aumenta l'umidità del combustibile;
  - una linea tagliata trasforma la vegetazione in non combustibile;
  - un argine cambia la quota.

  Una difesa deve ridurre l'esposizione **reale**. Non basta un contatore del tipo
  «mezzo vicino a casa, quindi difesa».
- **Strati separati per bersagli diversi.** Le case su celle non combustibili non bruciano
  nell'automa: serve uno strato di esposizione. Le persone all'aperto e le strade hanno un
  altro strato, il pericolo vicino al fronte. I due non sono intercambiabili.
- **Elementi dipinti come non combustibili** (strade, piazze) diventano immuni: una strada
  in mezzo alla vegetazione è pericolosa solo se il pericolo viene calcolato *vicino* al
  fronte e non dalla cella stessa.
- **Tarature in un solo punto** (`Tuning`). I valori predefiniti sono le costanti di
  sempre, e ogni meccanismo nuovo ha un interruttore.
- **Un meccanismo spento deve essere inerte**, anche a T+0. Va verificato con N corse
  identiche, prima e dopo, su tutti i territori: una derivazione giusta su un territorio
  può essere sbagliata sul successivo.

- **Comportamento delle squadre come grafo** (albero di comportamento o simile), dati e
  non codice. Regole che evitano bug difficili da vedere:
  - ogni nodo risponde riuscito, fallito o *in corso*; un'azione in corso **riprende** al
    passo dopo, e si azzera solo quando il genitore abbandona quel ramo;
  - le condizioni di sicurezza («il fronte è vicino: ritirarsi») vanno **ricontrollate a
    ogni passo** anche mentre un'azione è in corso (sequenza reattiva o interruzione da
    priorità più alta), se no la squadra resta sotto il fuoco;
  - un tempo di attesa (cooldown) non si azzera cambiando ramo;
  - se si sceglie per punteggio (utilità), ogni criterio va normalizzato tra 0 e 1, e
    l'azione in corso ha un piccolo bonus, così la squadra non oscilla tra due scelte
    quasi pari;
  - il grafo si valuta al ritmo del modello (tempo simulato), non del frame, e senza
    allocare a ogni passo; il ramo attivo si mostra sullo schermo mentre si tara.

## 4. Previsione per il giocatore

- Si usa un **ensemble dallo stato presente** e dal bollettino (N realizzazioni, isocrone
  a una certa percentuale), mai il futuro della partita.
- Si smussano le **polilinee** delle isocrone, non il raster.
- Il campo dei tempi di arrivo si può usare direttamente nella grafica, filtrato e
  ricampionato: aree, onde animate.

## 5. Reti e percorsi

- Il nodo carrabile più vicino spesso è un'isola della rete: si usa il più vicino
  **raggiungibile**.
- Mai un A* a ogni sotto-passo. Si ricalcola quando cambia l'obiettivo o a intervalli fissi.
- Una strada «chiusa» è un tratto non percorribile da un mezzo tra due incroci liberi, non
  una cella calda. Le strade si riaprono quando il pericolo cessa.

## 6. Trappole specifiche dei dati e dei modelli

- **Innesco di una sola cella:** in circa il 20 % dei casi non attecchisce. Si accende
  sempre una piccola area (patch).
- **Direzione del vento:** verificare se il parametro indica *da dove* soffia (in
  PROPAGATOR `w_dir` è la provenienza, e il sorgente del kernel trae in inganno).
- **Griglie sparse tramite NaN:** se nel modello «nessun cambiamento» è NaN, una griglia di
  zeri riclassifica tutta la mappa.
- **Unità delle condizioni al contorno:** verificarle (in PROPAGATOR l'umidità aggiunta è
  in punti percentuali, circa 30 punti per L/m²).
- **Risoluzione:** la griglia del modello (es. 20 m) vincola solo il modello. Vettori e
  agenti stanno in metri in un sistema locale, e il terreno del rendering è più fine e
  liscio, derivato da quello del modello e non il contrario.

## 7. Esempio: Isola di Roccavento

Crate `propagator-core` (CA), `fire` (esposizione, pericolo, ensemble), `abm` (rete,
guida, spegnimento), `rocca` (il `Game`, CLI e test), `game` (Bevy). Test chiave:

- `the_speed_does_not_change_the_game`;
- `orders_only_in_pause_and_one_evacuation_at_a_time`.

Laboratorio con interruttori in `Tuning`: `docs/05-LABORATORIO.md`.

---

Le regole sui grafi di comportamento del §3 adattano `ai-behavior-trees-utility-ai` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0). Nessun testo copiato.
