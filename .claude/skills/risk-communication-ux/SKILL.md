---
name: risk-communication-ux
description: Progettare interfaccia e segni sulla mappa di un gioco di rischio naturale per un pubblico generico. Usare quando si mostrano previsioni e incertezza (isocrone, tempi di arrivo, aree a rischio), stato di borghi e squadre, percorsi di evacuazione, strade chiuse, ordini con trascinamento/clic/touch/gamepad, tutorial guidato, chat dei personaggi, avvisi fuori schermo o la schermata finale.
---

# UX per comunicare un rischio

Il giocatore deve capire in pochi secondi **dove va il pericolo, quanto tempo c'è, che cosa
può fare**. Il mondo in 3D è il palcoscenico: gli elementi di gioco devono distinguersi
nettamente dal paesaggio.

## 1. Principi

- **Meno testo.** Icone, anelli, numeri, colori. Un'informazione si mostra una volta, nel
  posto giusto: niente doppioni tra pannelli e mappa.
- **Gli ordini si danno sulla mappa**, non in una barra laterale: si trascina la squadra,
  si clicca il borgo e si conferma. La barra, se c'è, dà solo informazioni.
- **Anteprima prima dell'ordine:** durante il trascinamento o il passaggio del mouse si
  vedono percorso e minuti, e la destinazione diventa rossa se non è valida (con un
  motivo in due parole: «⛔ no strade»). Se si rilascia vicino a un posto valido, l'ordine
  si aggancia a quel posto.
- **Una palette di significati, fissa:** rosso per il pericolo e il fuoco (sfumature per
  l'orizzonte temporale), verde per l'evacuazione e la via di fuga, colori distinti per le
  squadre. Lo stesso colore vale per un oggetto su badge, anello, percorso e area.
- **Pausa tattica:** in pausa si desaturano terreno, vegetazione e mare, e restano accesi
  UI, case e mezzi. Va desaturato poco, non grigio.

## 2. Segni sulla mappa (overlay)

- Materiale dedicato: colore pieno e saturo, bordo scuro largo, opacità alta, senza luce né
  nebbia, **sopra la chioma** (+20 m o più). Le linee drappeggiate vanno ricampionate
  ogni circa 5 m.
- La scelta migliore è **disegnarli nello shader del terreno** (strade evidenziate, strade
  chiuse da incrocio a incrocio, aree raggiungibili, aree d'intervento, aree previste), non
  con geometria sovrapposta. Si evitano i segmenti staccati e lo z-fighting, e i segni
  seguono il terreno.
- **Previsione:**
  - poche isocrone (+1 h, +3 h), sottili, in sfumature dello stesso rosso, con etichette che
    non si sovrappongono;
  - oppure aree riempite dallo shader, con un'onda animata guidata dal campo dei tempi di
    arrivo filtrato e ricampionato.
- **Zone d'azione:** cerchi semplici e non insiemi di esagoni. La griglia degli ordini resta
  una regola interna. Si anima il **movimento**, non il pulsare del colore. Niente
  tratteggi fini, che non si leggono.
- **Icone che non si sovrappongono mai:** si spostano sullo schermo e si collegano con
  una linea alla loro entità. Durante un trascinamento le altre icone si fanno da parte.

## 3. Stato e allarmi

- Il borgo mostra in un unico contenitore il nome, un **indicatore di vita** circolare
  (scende con le case colpite, da verde a giallo a rosso) e l'indicatore dell'evacuazione
  (timer circolare fino all'arrivo dell'ultima famiglia, poi una spunta). Da evacuato
  diventa verde.
- Se una squadra è in pericolo, il badge lampeggia in rosso con due parole («si ritira»).
- **Fuori dalla vista:** una freccia sul bordo dello schermo con icona e nome. Il clic
  porta la camera lì.
- **Chat di gruppo** breve: ritratto piccolo, ora, emoji, solo i messaggi utili in un
  contesto operativo. Arriva solo mentre il tempo scorre, non durante gli ordini. Sul
  telefono diventa una notifica.
- **Condizioni:** vento e umidità grandi in un angolo, la freccia del vento ruota con la
  camera, e in pausa il cambio di vento previsto.

## 4. Tutorial guidato

- Si svolge sullo stesso territorio e parte in pausa come la partita. Ogni passo ha una
  sola azione richiesta, e il gioco accetta solo quella.
- A ogni passo la **camera vola** sull'oggetto di cui si parla, e l'elemento da toccare
  **pulsa** (gli utenti non vedevano il punto indicato).
- Un fumetto grande con poco testo: **azione** in un colore, __luogo__ in un altro,
  «Avanti» solo nei passi di lettura.
- Se il tutorial mette in pausa da solo, dice che nella partita vera la pausa spetta al
  giocatore. I testi si aggiornano a ogni cambio della UI («in alto c'è il vento» quando
  ormai sta in basso a destra è un errore ricorrente).

## 5. Ingressi

- Mouse, trackpad e touch. Sul touch il primo tocco mostra l'anteprima e il secondo dà
  l'ordine. In alternativa al trascinamento: clic sulla squadra, poi clic sulla
  destinazione; Esc annulla.
- **Gamepad come mouse virtuale**, con un riquadro dei comandi del momento. Se c'è un pad
  collegato ma si usa il mouse, il cursore deve restare visibile.
- Le scorciatoie da tastiera rispettano il campo di testo attivo (in egui
  `wants_keyboard_input()`). Lo spazio mette in pausa.
- Cliccare una squadra o un borgo con il mondo in corsa lo ferma (autopausa). Dato
  l'ordine, il gioco riparte da solo.

## 6. Finale

Stelle grandi per categoria, con che cosa contano, il confronto con «senza ordini» in
numeri, il verdetto di un personaggio, un consiglio per riprovare, la classifica, i
messaggi ufficiali per i cittadini, «Riprova» e «Nuova partita». Non si attribuiscono
cause agli ordini senza una traccia che le dimostri.

## 7. Leggibilità, accessibilità e risposta agli eventi

- **Scala, non pixel fissi:** l'interfaccia si àncora ai bordi e agli angoli e si scala da
  una risoluzione di riferimento (es. 1920×1080). Per le forme diverse (ultrawide,
  telefono verticale) si sceglie una regola: si allarga lo spazio e gli angoli restano
  agli angoli.
- **Testo:** circa 18 px a 1080p come minimo su computer, di più per schermi visti da
  lontano; un'opzione **«testo più grande»** che regge fino al 200 %. Le traduzioni sono
  più lunghe dell'italiano (tedesco, finlandese): i contenitori si adattano al testo, e
  si controlla almeno una lingua lunga.
- **Margini sicuri** su telefono (notch, angoli arrotondati) e su TV.
- **Lo stato non si dice solo col colore:** verde/giallo/rosso del borgo vanno con una
  forma o un'icona (spunta, fiamma, timer), per chi non distingue rosso e verde.
- **Lampeggi e scosse:** brevi, rari, solo per gli eventi importanti, e con un'opzione
  «meno lampeggi / meno movimento» nelle impostazioni. Nessun lampeggio rapido a tutto
  schermo.
- **Risposta agli eventi a livelli:** ogni evento ha un livello (piccolo, medio, grande)
  che fissa insieme suono (`serious-game-audio`), animazione, durata e se la camera si
  muove. Le animazioni esagerano per poco e tornano a riposo, con un'uscita morbida (non
  lineare), e non bloccano mai l'input. Si muovono la grafica e la camera, mai lo stato del
  gioco.
- **Schermate come pila:** intro, tutorial, partita, impostazioni, finale si impilano;
  la schermata in cima riceve l'input, «Indietro»/Esc toglie quella in cima. Meglio di una
  serie di flag booleani.
- **Focus:** ogni schermata che si apre ha un elemento già selezionato, così il gamepad
  (o la tastiera) non resta bloccato; mouse e focus convivono, e cambiare dispositivo non
  lascia mai il giocatore senza cursore. I suggerimenti dei comandi seguono l'ultimo
  dispositivo usato.
- **L'interfaccia si aggiorna quando il dato cambia**, non ricostruendo ogni testo a ogni
  frame.

## 8. Verifica

Ogni modifica visiva si guarda in immagine: screenshot di stati arbitrari, senza giocare
(in Roccavento `KIOSK_SNAP`: vedi `CLAUDE.md`). Si scatta **anche non nadirale**, da più
angoli, e con la UI spenta (`ui=0,marks=0,iso=0`) quando si giudica la scena. Si
controllano desktop e telefono, verticale e orizzontale, almeno una lingua lunga e il
testo grande; una prova si fa con il solo gamepad. Nel resoconto si dice quali
risoluzioni sono state davvero provate. Le schermate di riferimento di Roccavento sono
descritte in `docs/01-GIOCO.md` §6.

---

Fonte del §7: adattato da `game-ui-ux`, `game-feel` e `input-systems` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0); la soglia del testo vi è citata dalle
Xbox Accessibility Guidelines (101). Nessun testo copiato.
