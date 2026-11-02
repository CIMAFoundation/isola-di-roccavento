# 01 — Il gioco: Isola di Roccavento

Regole **in vigore** dal 2026-10-14 (nuovo progetto). Prevalgono sul codice: se non coincidono, segnalarlo. Ogni cambio di regola è una decisione dell'utente, da scrivere qui con la data (lo storico è nella storia git).

## In una frase

Uno strategico in tempo reale, sul modello di StarCraft, per chiunque apra la pagina web o scarichi la versione desktop (dal 2026-10-19 non è più un chiosco presidiato). Un incendio simulato minaccia i borghi di un'isola. Il giocatore **mette in pausa** per chiedere la previsione all'esperto, poi **sposta le squadre** su una griglia esagonale e **evacua i borghi**, uno alla volta. La simulazione decide gli esiti. Poche parole: la comunicazione (personaggi, messaggi, debriefing) si riscrive in un secondo momento.

**Obiettivo educativo:** incertezza, tempi, risorse limitate, adattarsi. Non è un addestramento e non insegna procedure operative.

## 1. Struttura della partita

1. **Apertura:** nome del giocatore, «TUTORIAL» (prova guidata) o «INIZIA» (salta alla partita) e la classifica di oggi.
2. **Tutorial** (`rocca::tutorial`): un incendio piccolo vicino a Le Ghiande, con dinamiche reali. **Parte in pausa**, come il gioco vero (2026-10-14). Passi: la scena e la previsione («Avanti»); la vita dei borghi (anello che si svuota con le case raggiunte dal fuoco, da verde a giallo a rosso; «Avanti», 2026-10-15); i due ordini, trascinando il badge dei vigili di Castelvento e quello dei volontari di Pian dei Grilli sulla destinazione indicata (nel testo non si parla di esagoni: il giocatore non li vede più, 2026-10-19); spiegazione della zona coperta (contorno giallo, nessun esagono pieno; «Avanti»); «Riprendi»; il mondo corre da solo per **30 min simulati** (15 s reali a ×120, `tutorial::RUN_MIN`) e **si ferma da solo**, e il testo dice che nella partita vera la pausa la deve premere il giocatore; evacuazione di Le Ghiande; «Riprendi» **fino alla fine dell'evacuazione**: il tutorial finisce lì (2026-10-14), non a fuoco spento; la barra del tempo dura quanto il caso `tutorial` (60 min, la fine reale è alle 11:51). Il gioco accetta **solo** l'azione richiesta e il mondo resta fermo finché il giocatore non la compie, quindi l'esito è scriptato. Senza ordini, nei 60 min colpisce 3 case e 20 persone.
3. **Partita:** un incendio da affrontare (`game.json` → `game`, oggi il caso `medium`, 5 ore). Il controllo passa al giocatore. La partita **parte in pausa**, senza limite di tempo, per esplorare la scena e dare i primi ordini; «Via!» fa partire il tempo e non consuma l'esperto (2026-10-14). L'incendio è **segnalato in ritardo**: quando la partita comincia brucia già da `start_min` minuti (2026-10-14).
4. **Fine:** stelle e classifica del giorno.

## 2. Il tempo e la pausa

- Nessun turno. Il tempo scorre a **×120** o è **fermo**, nient'altro (2026-10-15, richiesta dell'utente; prima ×60): 1 s reale vale 2 min simulati (`rocca::RUN_X`).
- **Pausa = chiedere all'esperto scientifico.** In pausa il **mondo si ferma** (2026-10-14: «pausa è stop, non tempo rallentato»). La pausa **non ha limite di tempo** (2026-10-14, richiesta dell'utente): finisce quando il giocatore preme «Riprendi». **Nessuna attesa tra una pausa e la successiva** (2026-10-16, richiesta dell'utente: tolti `PAUSE_COOLDOWN_REAL_S` e `PAUSE_COOLDOWN_S`).
- **In pausa il piano si può cambiare:** una squadra si sposta di nuovo trascinando la sua icona; un'evacuazione ordinata nella stessa pausa si annulla, oppure si sostituisce scegliendo un altro borgo (`Game::cancel_evacuation`). Dopo la ripresa no (2026-10-14).
- **Tutti gli ordini si danno solo in pausa:** spostare le squadre ed evacuare.
- In pausa compaiono:
  - la **previsione**: isocrone al 50 % di 16 simulazioni dal fuoco attuale, a +1 e +3 h (la +2 h tolta il 2026-10-16), in **due sfumature di rosso** (+1 h la più intensa, +3 h la più chiara), sottili e con l'etichetta («tra 1 ora»…; 2026-10-16). Le etichette non si sovrappongono sullo schermo, senza interventi (`rocca::forecast`);
  - il **bollettino meteo**: vento attuale e cambio annunciato;
  - l'**umidità del combustibile**.
- Durante il tutorial il tempo è fermo mentre un passo aspetta il giocatore.
- La partita finisce quando il fuoco è spento e nessuno è in strada, oppure alla durata del caso.

## 3. Le squadre

- **Una squadra** è un'autobotte con 3 persone. Sono tutte squadre **AIB**, senza distinzione di colore o di nome (2026-10-16): «Squadra A»…«Squadra E», in giallo. 3 stanno al presidio di Castelvento, il borgo più grande, e 2 al presidio di Pian dei Grilli, un borgo minore a ovest, dall'altra parte dell'isola rispetto a Castelvento (`game.json`: `stations`, `roster`).
- Si spostano su una **griglia esagonale** di 150 m tra centri adiacenti (`rocca::hex`; 2026-10-15, dimezzati da 300 m). Si può scegliere solo un esagono attraversato da una strada carrabile della rete principale: la squadra guida fino al punto stradale dell'esagono più vicino al centro.
- Ogni squadra ha un **badge** (icona dell'autobotte e lettera) sopra il punto dove è assegnata, con un anello a terra nel colore della squadra: sull'esagono dell'ultimo ordine, oppure alla sua sede (`kiosk::pins`). Non c'è nessun cono (2026-10-17). Il veicolo non ha icone sopra (2026-10-14).
- In pausa si **trascina l'icona della squadra** sull'esagono scelto: durante il trascinamento il gioco mostra **percorso e tempo di viaggio**, che comprende il **rientro a piedi delle persone alla camionetta** (parte solo quando tutti sono a bordo; 2026-10-17, `MovePlan::regroup_min`), evitando le strade chiuse dal fuoco (`Game::plan_move`). **Lasciata l'icona, l'ordine è dato**, senza conferma (2026-10-14). Anche un clic sul badge e poi sull'esagono funziona: con la squadra scelta, l'esagono sotto il puntatore è anticipato con contorno, strada e minuti; **Esc**, o un clic dove la squadra già sta, annulla (2026-10-16, collega).
- Il badge della squadra mostra la **lettera** (non un numero) e **lampeggia in rosso** quando è in pericolo (`Game::team_in_danger`); si ritira da sola (regola già in vigore, `UnitState::Withdrawing`). Ogni squadra ha un **colore** (blu, arancio, magenta, giallo, bianco; `marks::TEAM_COLOURS`), lo stesso su badge, anello a terra, strada di anteprima e **cerchio** della sua zona. Il rosso resta al pericolo, il verde all'evacuazione (2026-10-16). Una squadra in pericolo mostra la scritta «fuoco vicino!» / «si ritira» / «fuori servizio»; né l'anello rosso pulsante sul mezzo né la linea fino al badge (tolti il 2026-10-17). La strada percorsa dai mezzi in marcia non è evidenziata: si vede solo l'anteprima mentre si sceglie l'ordine.
- **Ripartenza automatica:** nella partita il mondo riprende da solo appena un ordine (spostamento o evacuazione) è dato (2026-10-17, richiesta dell'utente). Nel tutorial resta il passo «Riprendi». Per dare un altro ordine si clicca di nuovo una squadra o un borgo (autopausa).
- **Autopausa:** cliccare una squadra o un borgo con il mondo in corsa lo ferma (2026-10-16, collega; non nel tutorial).
- **Fuori dalla vista:** un borgo che perde case, o una squadra in pericolo, compaiono come **freccia sul bordo** dello schermo (icona, nome, colore giallo/rosso); un clic porta la camera lì (`ui::offscreen_alerts`).
- **Borghi:** da lontano i borghi tranquilli sono solo un anello con una casa (il nome al passaggio del mouse); il cartellino è blu se intatto, giallo o rosso se colpito, **verde con «evacuato»** a evacuazione finita.
- **Chat:** solo ciò che serve a decidere: squadra che si ritira o è persa, prime case e una casa su tre colpite in un borgo, evacuazione interrotta o completata, vento che gira; 4 righe (`chat::Kind`).
- **Aree e posti (2026-10-17):** la griglia degli ordini è fitta (esagoni di 75 m, `HEX_M`) e l'area d'intervento di una squadra è un **cerchio di 300 m** (`TEAM_AREA_M`) attorno al suo punto, non un insieme di esagoni. Il gioco disegna il cerchio nel colore della squadra, un cerchietto dove andrà (rosso se rifiutato) e, molto tenue, una fascia lungo le strade raggiungibili (gli esagoni restano solo la regola interna). Sul **touch** (niente passaggio del mouse) il primo tocco mostra l'anteprima, il secondo sullo stesso punto dà l'ordine.
- Arrivata, la squadra **attacca il fuoco come può** (2026-10-14), con il modello di spegnimento conservato (`abm::suppression`):
  - l'**autobotte** aumenta l'umidità del combustibile entro **100 m** da dove sta (acqua, manichetta). Durante l'intervento si sposta **sulle strade degli esagoni di competenza** (il suo e quelli intorno) verso il combustibile minacciato, senza avvicinarsi a meno di 80 m dalle fiamme (2026-10-15, richiesta dell'utente);
  - l'**autobotte va solo su strade carrabili**; le persone viaggiano a bordo, scendono dove si ferma e vanno **a piedi in linea retta** (1,1 m/s, rallentate dalla pendenza) al lavoro. Sono unità **separate**: si dispongono a ventaglio di 60 m (`CREW_SPREAD_M`) intorno al punto stradale e ognuna taglia la propria linea. A un nuovo ordine chi è a piedi torna all'autobotte, che parte quando sono tutti a bordo (2026-10-15, richiesta dell'utente);
  - le **persone a piedi** tagliano la vegetazione di traverso alla via del fuoco. Finita una linea, ne tagliano un'altra più vicina al fuoco, entro **2 esagoni** intorno (300 m) dal punto stradale dell'esagono (`CREW_RANGE_M`; 2026-10-15, prima 400 m).

  Viaggiano insieme all'autobotte e insieme ricevono gli ordini. La squadra **protegge anche le case** entro 300 m (`PROTECT_M`). Quando l'autobotte ha finito l'acqua **resta dov'è** e la ricarica è implicita: torna piena dopo **10 min simulati** (`REFILL_S`); intanto continua a proteggere le case vicine (2026-10-14, prima andava agli idranti). Si ritira se il pericolo è troppo alto e può essere persa.
- Evacuazione e difesa sono **slegate**: le squadre non evacuano e l'evacuazione non muove squadre.

## 4. L'evacuazione (modello semplificato)

Non c'è più il modello individuale delle persone (IBM). Ogni casa è una famiglia con un numero di persone (`rocca::evac`).

- **Un borgo alla volta.** Il successivo si può evacuare quando l'ultima famiglia del borgo precedente è partita.
- **Passando sopra il borgo** il gioco mostra già il **percorso** (tratteggiato) verso l'area sicura più vicina fuori dal borgo e il **tempo**: fino all'arrivo dell'ultima famiglia all'area sicura, cioè preparazione (5 min), partenze e viaggio. Si mostrano solo persone e minuti, senza le parti né la destinazione (2026-10-14, semplificazione). Poi si clicca il borgo e si preme «Evacua» nel riquadro.
- **Aree di raccolta scelte a mano** (2026-10-14): ogni borgo (tranne Le Ghiande) ha la sua, facile da raggiungere e fuori dal bosco del valico (`evacuation.refuges` in `game.json`, elenco in `02-TERRITORIO.md`). Se la strada è chiusa dal fuoco, o il borgo non ne ha una, vale l'area sicura automatica più vicina. Senza l'elenco il meccanismo è inerte.
- **Limite rigido:** al massimo **6 persone al minuto** escono dal borgo (`evacuation.per_min`). Le famiglie più vicine al punto di partenza partono per prime. In auto si viaggia a 8 m/s.
- **Strade chiuse dal fuoco:** sono segnate in rosso sulla mappa. Ogni minuto il percorso viene ricontrollato.
  - Se il fuoco taglia la strada, si cerca un altro percorso.
  - Se non ce n'è nessuno, l'evacuazione si **interrompe** e chi è ancora a casa passa in **autoprotezione**.
  - Una famiglia già in strada con la via tagliata cerca un'altra strada, altrimenti si ferma.
- **Quando il fuoco raggiunge le persone** (stessa regola per lo schermo e per le stelle):
  - a casa senza ordine: il pericolo alla porta arriva a `ALARMING`, oppure la casa è colpita;
  - in autoprotezione: la casa è colpita, oppure il pericolo alla porta arriva a `IMPASSABLE`;
  - in strada: il pericolo dove si trovano arriva a `IMPASSABLE`.

## 5. Stelle e classifica

- **Tre stelle da 1 a 5** (`rocca::score`), ognuna come quota risparmiata rispetto allo stesso incendio senza ordini:
  - **Persone:** persone non raggiunte dal fuoco;
  - **Case:** case non colpite;
  - **Ettari:** ettari non bruciati.

  Le soglie sono provvisorie. Dal 2026-10-16 il finale spiega quantità e percentuale del danno evitato, distingue peggioramenti e propone cosa osservare alla prossima prova; include tre messaggi di protezione civile per i cittadini, con fonte ufficiale, anche offline. La scala più generosa è pronta **solo come proposta**, con prove in [`04-VALUTAZIONE.md`](04-VALUTAZIONE.md): le soglie del gioco non cambiano prima della scelta dell'utente.
- **Classifica del giorno:** prima le stelle Persone, poi Case, poi Ettari; a pari merito vince chi ha giocato prima (2026-10-14). Compare in apertura e nel finale. L'operatore la azzera.

## 6. Interfaccia

- La mappa occupa quasi tutto lo schermo; il rendering 3D del diorama è conservato.
- **Pausa:** un pulsante rotondo e grande, in basso al centro, **dentro il pannello del tempo** (2026-10-16: prima in alto a sinistra) (giorno e ora, barra di avanzamento grande). Pronto: anello ambra e simbolo di pausa. In pausa: simbolo di avvio, anello verde pieno, nessun conto alla rovescia. In gioco: sempre pronto, nessun conto alla rovescia (2026-10-16). In generale si usano icone, anelli e numeri al posto delle frasi.
- **Barra in alto:**
  - giorno e ora;
  - vento e umidità;
  - vento (freccia vista dalla camera) e umidità (goccia); in pausa la legenda delle isocrone e il cambio di vento previsto (orologio, ora, freccia).
  - in pausa, la spiegazione della previsione e il meteo.
- **Barra laterale:** solo informazioni, nessun ordine: l'elenco delle squadre con il loro stato; l'elenco dei borghi; i conteggi. Gli ordini si danno sulla mappa (2026-10-14).
- **Sulla mappa:**
  - icone delle squadre, dove sono assegnate, da trascinare (in pausa), e etichette dei borghi: passandoci sopra, percorso e persone e minuti dell'evacuazione; al clic il riquadro con persone, minuti e «Evacua» / «Annulla»;
  - sul borgo in evacuazione, un **timer circolare**: l'anello si svuota col tempo che resta fino all'arrivo dell'ultima famiglia, con i minuti al centro. Parte dai minuti annunciati e scende di un minuto per minuto simulato: annuncio e timer vengono dalla stessa regola (`Evacuations::remaining_min`, 2026-10-14); a fine evacuazione una spunta, se il fuoco l'ha interrotta un avviso;
  - la **chat di gruppo** (2026-10-16): in alto a sinistra (2026-10-16, spostata dall'utente), righe brevi e colloquiali con ritratto piccolo, ora ed emoji, dette da squadre, sindaca ed esperto su ciò che accade (partenza, arrivo, acqua finita, ritirata, evacuazione partita/deviata/completata, vento). Sul telefono compare solo come notifica in alto. Il testo viene dal nucleo (`Game::chat`);
  - i segni sulla mappa (esagoni, percorsi, strade chiuse, isocrone) hanno un **materiale loro** (`flat`): colore pieno e acceso, senza luce né nebbia, con un bordo scuro, disegnati sopra la vegetazione (2026-10-14);
  - esagoni, in pausa o con una squadra scelta;
  - esagono scelto e percorso, tratteggiati in bianco;
  - strade percorse dalle squadre, in ciano;
  - evacuazione, in **verde** (il colore della via di fuga, diverso dal ciano delle squadre): tratteggiata se proposta, continua se in corso; lo stesso verde nel riquadro e nel timer (2026-10-14);
  - strade chiuse, in rosso;
  - isocrone della previsione e freccia del vento.
- **Avvisi:** l'ultimo evento, in una riga.
- **Schermate** (`kiosk::screens`, `kiosk::characters`; stile e idee del vecchio chiosco, 2026-10-14). A parlare sono **volti** in fumetto, con il ruolo sopra: la **volontaria** (guida: apertura, tutorial, finale), l'**esperto** e i **vigili del fuoco** (solo nella riga delle persone con cui si lavora):
  - **apertura:** carta centrale con logo CIMA e titolo, la volontaria che dice che cosa è in gioco (squadre e borghi), «Le persone con cui lavori», «Come ti chiami?», «Inizia» e «Salta la prova guidata», avvertenza sul territorio immaginario; accanto, «Classifica di oggi»;
  - **tutorial:** il fumetto della volontaria in alto, al centro (2026-10-16, prima in basso: ora lì sta il pannello del tempo; grande, testo breve; `**azione**` in arancione e `__luogo__` in blu, 2026-10-15), con il passo (n/N), «Avanti» nei passi di sola lettura e «tocca a te» negli altri; a ogni passo la **camera vola** a ciò di cui si parla (l'innesco, la squadra e l'esagono, il borgo) e per 7 s una **cornice ambra** lampeggiante evidenzia ciò che si deve toccare (pulsante della pausa in basso al centro, bandiera e esagono, borgo); un trascinamento della mappa interrompe il volo; la mappa resta usabile, ma il gioco accetta solo l'azione richiesta; «Salta il tutorial» o Esc;
  - **finale:** «Com'è andata, <nome>», le stelle grandi di Persone, Case, Ettari con che cosa contano, il verdetto della volontaria e il confronto con il fuoco senza ordini; a destra il posto in classifica, la riga di avvertenza sulla realtà («quando arriva l'ordine di evacuazione si parte subito») «Riprova» (stesso incendio e stesso seme, da capo, 2026-10-19) e «Nuova partita».
- **Web su telefono** (2026-10-15, richiesta dell'utente): supporto verticale e orizzontale, profilo grafico leggero e memoria ridotta; pannelli e messaggi compatti e leggibili, controlli touch con trascinamento delle squadre e alternativa «tocca squadra, poi destinazione». Dettagli e verifiche in `web.md`. Le regole della partita restano condivise.
- **Controlli:**
  - mouse o trackpad; oppure **gamepad** (2026-10-17): croce sinistra/destra = squadra precedente/successiva, su/giù = borgo precedente/successivo (la camera vola sulla scelta; a sinistra un elenco evidenzia la voce scelta); con una squadra scelta in pausa il mirino è il centro dello schermo e si sposta muovendo la camera (anteprima come col mouse), A manda la squadra; con un borgo scelto X evacua; Y = ferma/riprendi il mondo; B = Esc. Camera: stick sinistro = muove la mappa, stick destro = ruota e inclina, LT/RT = zoom, RB = tutta l'isola. Fuori dal gioco (pulsanti, menu operatore), o tenendo LB, lo stick sinistro è un cursore e A un clic. Con un pad collegato un riquadro in basso a destra elenca i comandi del momento. Mario tiene i suoi comandi;
  - trascinare per spostare la mappa, un clic senza trascinare per scegliere;
  - rotella o pizzico per lo zoom, tasto destro o Shift + trascinare per ruotare;
  - Home per la vista iniziale.

  Funziona offline, in italiano, e si gioca anche senza audio: ogni suono accompagna qualcosa che si vede già.
- **Sequenza d'apertura** (richiesta dell'utente, 2026-10-30; `screens::opening`): al lancio, il logo CIMA su nero con il culmine del jingle (la scena non è disegnata finché c'è il nero, `view::curtain`); poi il logo sfuma e il nero si apre sull'isola sulla cadenza del jingle, la camera arriva dal mare come da un aereo, veloce già sotto il nero e sempre più lenta, quasi orizzontale verso il sole che sorge, e solo alla fine scende all'inquadratura del titolo; titolo e «Gioca» compaiono sulla fine della cadenza (3,6 s), il volo finisce a 8 s e la regia riprende la camera piano, in 4 s, senza stacchi né dissolvenze (utente, 2026-10-30). Un clic o un tasto la salta; su telefono non c'è. Fino a «Gioca» la luce è un'**alba fissa sul mare** (`sky::Dawn`): il mondo scorre, l'ora del sole no (sole a 11° sull'orizzonte, colori di un sole più alto: dorati, non rossi); poi la luce torna all'ora della partita in 2,5 s.
- **Orizzonte** (2026-10-30, `horizon.rs`): cinque isole a 19–28 km e una costa di terraferma a nord a 30 km, sagome semplici color foschia.
- **Godrays** (2026-10-30, solo profilo «alta», `godrays.rs`): con il sole inquadrato, ciò che è luminoso attorno al sole si allunga verso di esso (pass a schermo intero prima del bloom).
- **Audio** (richiesta dell'utente, 2026-10-30; `crates/game/src/sound.rs`):
  - **ambiente** che si sente **solo con la camera vicina** (pieno sotto 400 m, nullo sopra 1,5 km) e resta sotto i segnali: fuoco (celle che bruciano vicino al punto inquadrato, più forte dove l'intensità è alta), vento molto leggero (circa un terzo del fuoco), risacca (mare attorno al punto inquadrato). **Fuoco e vento sono registrazioni** (utente, 2026-11-02; Freesound, CC0), ognuna in due versioni che si dividono il livello: fuoco di macchia e fronte che rugge (secondo la lunghezza media della fiamma vicino alla camera), e il fuoco si sente dal lato dello schermo dove brucia di più (mai tutto in un orecchio; utente, 2026-11-02; il vento invece non dipende dalla direzione della camera), vento tra gli alberi e in campo aperto (secondo quanto bosco ancora in piedi c'è attorno al punto inquadrato; il vento forte le suona un po' più alte). La risacca resta sintetizzata (`crates/sfx`). Si sente anche con Mario; **cicale** e **gabbiani** registrati (Wikimedia Commons, pubblico dominio/CC0): le cicale lontano dal fuoco, tre registrazioni che si alternano; un gabbiano ogni tanto quando si guarda il mare. In pausa l'ambiente si attenua e si ovatta;
  - **segnali sintetici**, niente suoni realistici (sirena e campane tolte, utente 2026-10-30): pausa e ripresa (bip radio), squadra presa e ordine dato, avviso o rifiuto, passo del tutorial, evacuazione avviata e finita, case colpite, persone in pericolo, squadra che si ritira, il vento che gira (dalla chat), tre note alla fine; clic dei pulsanti (Kenney, CC0);
  - **jingle** in due tagli scelti dall'utente sull'inviluppo (2026-10-30): il culmine in Mi minore (2,6 s) mentre il logo CIMA compare, la cadenza finale su Sol (2,7 s) mentre il logo sfuma sull'isola; chi salta l'apertura prima del logo sente solo la cadenza (generato con Lyria 3, variante «archi leggeri»). Dal 2026-11-02 (utente): il culmine parte 0,3 s dopo l'avvio, la cadenza 0,6 s dopo (0,9 fino alla seconda richiesta dello stesso giorno, «diminuisci ancora la distanza»: il culmine cala già a 0,8 s), senza vuoto in mezzo; i due tagli si programmano insieme sull'orologio reale e sul web li suona il browser (WebAudio, `window.roccaPlayAt` in `web/index.html`), perché l'audio del gioco, riempito dal thread principale, gracchiava nei primi frame lunghi. Logo e schiarita dell'isola restano dov'erano (il logo sfuma a 1,2 s, quando il culmine si spegne);
  - **mix** (2026-11-02; massima sonorità su 400 ms, volumi predefiniti): avvisi importanti (case colpite, persone in pericolo, squadra che si ritira, evacuazioni, stelle) −18 LUFS, radio della pausa −21, raffica −22; interfaccia sotto i segnali (−24…−28, clic −34); jingle −17; fuoco vicino al massimo −20, mare −24, vento al massimo circa −30/−33 (un terzo del fuoco; brezza tra le foglie e nell'erba, non raffiche: utente, 2026-11-02). Durante un avviso importante l'ambiente scende di 6 dB (in 50 ms, risale in circa mezzo secondo); lo stesso segnale non si ripete prima di 1 s; i suoni d'interfaccia variano di ±1 semitono e fino a −1,5 dB. I cursori sono percepiti: il guadagno è il quadrato del cursore;
  - audio acceso di default, pulsante 🔊/🔇 accanto a «Impostazioni», in Impostazioni muto e volumi di ambiente ed effetti; la scelta resta (`~/.rocca_ventosa/audio.txt` o il browser). Sul web parte al primo tocco o tasto (regola dei browser). File e licenze: `data/sounds/CREDITS.txt`, rigenerabili con `tools/sounds/prepare.py`.
- **Impostazioni** (F2 o «Impostazioni», utente 2026-10-21; prima menu operatore): lingua (elenco con ricerca, anche nelle schermate iniziali), vista iniziale, audio, livello grafico e antialiasing. Tolte le voci di debug (scenario, nuova visita, vai alla partita, seme, «Ferma il mondo», freecam, azzera classifica).
  Il selettore operatore si applica con «Nuova visita» o «Vai alla partita», senza interrompere la partita alla sola selezione. La scelta resta per le visite successive nella stessa sessione; al riavvio torna `medium`, lo scenario attuale predefinito. Tutorial invariato. Classifiche e confronto senza ordini usano il caso effettivamente giocato, anche se l’operatore ha già selezionato il successivo.
- **Nessun reset per inattività** (non c'è più un chiosco presidiato, 2026-10-19).
- **Mario** (easter egg, `docs/MARIO.md`): **scollegato** dal 2026-10-14. Usava le persone dell'IBM e i vecchi stati del chiosco. Da decidere se ricollegarlo.

## 7. Lingue (richiesta dell'utente, 2026-10-21)

- Il gioco parla 38 lingue d'Europa: le 24 dell'Unione più norvegese, islandese, catalano, galiziano, basco, lussemburghese, gallese, bosniaco, serbo, macedone, albanese, ucraino, bielorusso, russo e turco (elenco in `rocca::i18n::LANGS`). **L'italiano è la lingua dei sorgenti**: ogni testo è scritto in italiano nel codice con `tr!("…")` e quel testo è la chiave del catalogo `data/i18n/<codice>.json` (italiano senza catalogo, inerte).
- **Scelta** (2026-10-30): `ROCCA_LANG` (sul web `?lang=de`), poi l'ultima scelta del giocatore (`~/.rocca_ventosa/lingua.txt`, `localStorage`), poi **la prima lingua supportata tra quelle preferite** del browser (`navigator.languages`) o del sistema (`sys_locale::get_locales`), confrontando anche il solo prefisso (`de-AT` → `de`), altrimenti **inglese** (decisione dell'utente, 2026-10-30; l'italiano resta la lingua dei sorgenti). **Selettore**: nel titolo e nell'introduzione, in basso a destra, un pulsante (globo e bandiera in uso) apre un pannello di bandiere disegnate (`kiosk/flags.rs`, sei per riga, codice sotto ciascuna); un clic o un tocco cambia subito lingua. Durante la partita non c'è: l'angolo è delle condizioni meteo e la chat resta nella lingua in cui è nata; la lista resta in Impostazioni (F2), con la stessa scelta salvata. Bandiere per le lingue di più paesi: inglese → Regno Unito, catalano → Senyera, galiziano → Galizia, basco → Ikurriña, gallese → Galles; le altre del paese principale. **La lingua si legge quando il testo nasce**: righe di registro e messaggi della chat restano nella lingua in cui furono scritti; si sceglie prima di cominciare.
- **Nomi propri non tradotti**: borghi, nomi dei luoghi, nome del giocatore, titolo del gioco.
- **Easter egg: klingon** (`tlh`, trascrizione latina, **Ctrl+Maiusc+K** a tastiera libera, di nuovo per tornare indietro; non è nel selettore e non si salva). **Il klingon è un'approssimazione**: vocabolario canonico dove noto, per il resto parole ricomposte; da far rivedere a chi lo parla.
- **Nuovo testo = nuova chiave**: dopo aver scritto un `tr!` nuovo, `python3 tools/i18n/keys.py` aggiorna `data/i18n/keys.json` e `python3 tools/i18n/keys.py --check` dice quali cataloghi mancano o hanno segnaposto (`{0}`), `**`, `__` o a capo diversi. I valori si passano numerati: `tr!("{0} verso {1}", a, b)`; i formati (`{:.0}`) si applicano prima.
- **Fuori dalla traduzione** (restano in italiano): i passatempi di Mario (`mario*.rs`), i messaggi di errore di salvataggio e tecnici, il menu operatore tranne le voci già passate da `tr!`, la pagina introduttiva `web/index.html` (compresi i testi di caricamento).

## 8. Fuori dal gioco (per ora)

- Sindaca, preallerta, pattuglie porta a porta, coordinatore automatico: tolti il 2026-10-14. La comunicazione si riscrive dopo.
- Mezzi aerei e squadre a piedi: non previsti.
- Il territorio è dichiaratamente immaginario: nella realtà si seguono le indicazioni delle autorità.
