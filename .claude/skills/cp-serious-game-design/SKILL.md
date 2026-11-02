---
name: cp-serious-game-design
description: Progettare o rivedere un serious game di protezione civile su un rischio naturale (incendio boschivo, alluvione, frana, sisma, maremoto, ondata di calore). Usare quando si definiscono pubblico, obiettivo didattico, ruolo del giocatore, ciclo di gioco, durata, punteggio, finale o messaggi per i cittadini, e quando si valuta se una meccanica è credibile per la protezione civile.
---

# Serious game di protezione civile: progetto

Il gioco serve a far **capire** un rischio, non a far vincere il giocatore né ad addestrare
operatori. Ogni scelta di design si giudica con due domande:

1. *Che cosa capisce di più il giocatore dopo questa partita?*
2. *Un operatore di protezione civile la troverebbe credibile?*

Se una meccanica è divertente ma falsa (per esempio un'evacuazione che non costa niente,
o persone che scappano a piedi da case isolate), va cambiata la meccanica, non il testo
che la racconta.

## 1. Prima di scrivere codice: la scheda del gioco

Scrivere nel documento delle specifiche, con la data, le risposte dell'utente a queste
domande. Se non le ha date, chiederle (al massimo 4 alla volta):

| Tema | Domanda |
|---|---|
| Pubblico | Chi gioca, dove, quanto tempo ha? Pubblico generico, scuole, evento presidiato? |
| Obiettivo didattico | 2–4 concetti da portare a casa (es. incertezza, tempi, risorse limitate, adattarsi). |
| Cosa **non** si insegna | Di solito le procedure operative (spegnimento, soccorso): va scritto. |
| Ruolo | Chi è il giocatore? Un ruolo istituzionale reale (sindaco, DOS) obbliga a rispettarne poteri e limiti; spesso è meglio «chi aiuta» la squadra, con i personaggi accanto. |
| Rischio e scenario | Territorio immaginario o reale? Uno immaginario evita di insegnare cose false su un luogo vero; va dichiarato. |
| Leve del giocatore | Poche e distinte (es. spostare squadre, evacuare un borgo alla volta). |
| Durata | Partita di 3–5 minuti reali: da qui si ricava la compressione del tempo (vedi `hazard-sim-integration`). |
| Esito | Che cosa conta e in che ordine (es. persone, poi case, poi ettari). |
| Piattaforme | Web, desktop, telefono, touch, gamepad, offline, audio sì o no. |

## 2. Struttura che ha funzionato

- **Apertura** breve: titolo, che cosa è in gioco, il nome del giocatore (così i
  personaggi gli parlano), una classifica leggera.
- **Tutorial scriptato** sullo stesso territorio, con un evento piccolo ma con fisica reale.
  Il gioco accetta solo l'azione richiesta. Il tutorial finisce quando l'obiettivo del
  passo è raggiunto, non quando l'evento si esaurisce.
- **Partita**: un evento, che **parte in pausa** perché il giocatore possa guardare la scena.
- **Pausa = chiedere la previsione all'esperto.** È anche l'unico momento in cui si
  danno ordini. Così le decisioni sono poche e pesano, senza contare le pause.
- **Finale**: stelle per categoria come **danno evitato rispetto allo stesso evento senza
  ordini** (stesso seme), quantità e percentuale, un consiglio per riprovare, «Riprova»
  sullo stesso evento.

## 3. Realismo: le domande da fare a ogni meccanica

- **Chi fa che cosa, nella realtà?** Le squadre attaccano il fronte, non solo stanno
  davanti alle case. I mezzi vanno solo su strada e le persone scendono a piedi. Le
  famiglie evacuano in auto, salvo distanze minime.
- **Quanto tempo ci vuole?** Preparazione, partenze scaglionate (limite di persone al
  minuto), viaggio, rientro della squadra al mezzo prima di ripartire. Ogni tempo
  annunciato al giocatore deve essere quello simulato.
- **Che cosa non si sa?** La segnalazione arriva in ritardo, la previsione è un ensemble
  e non la verità, il vento può girare come annunciato dal bollettino. **Nessuna
  informazione dal futuro.**
- **Che cosa costa un errore?** Evacuare chi non era a rischio deve avere un costo
  *fisico*, nella simulazione (strade occupate, persone in strada vicino al fronte, tempo
  perso), e il finale lo spiega. Non si usano penalità artificiali sul punteggio.
- **Il territorio è credibile?** Niente aree di raccolta in mezzo al bosco, niente
  incroci a stella, niente borghi a scacchiera perfetta. Se una zona non viene mai
  raggiunta dall'evento, si cambia il territorio (vegetazione, un valico), non le regole.
- **Fonti.** Per le regole operative leggere documenti ufficiali (Dipartimento della
  Protezione Civile, Vigili del Fuoco, piani comunali) e annotare che cosa se ne ricava.
  Citare le fonti nel documento delle regole.

## 4. Linguaggio e comunicazione

- Testi brevi, colloquiali, nella lingua del pubblico. Al posto delle frasi si usano icone,
  anelli e numeri.
- Usare i termini tecnici corretti, spiegati una volta. Per esempio «spotting» e non
  «faville»; «area di attesa» e non «rifugio» se il piano dice così. Raccogliere i termini
  in un glossario nel documento delle regole.
- Personaggi con volto e ruolo (volontaria che guida, esperto, squadre, sindaca) e una
  chat breve con solo i messaggi utili in un contesto operativo: «siamo arrivati»,
  «evacuazione completata».
- Nel finale, 2–3 **messaggi ufficiali per i cittadini**, con la fonte e disponibili
  offline. Per gli incendi: conoscere il piano comunale, chiamare il 115/112 indicando il
  luogo, non ostacolare i soccorsi. La fonte è
  <https://iononrischio.protezionecivile.gov.it>.
- Un'avvertenza visibile: territorio immaginario, nella realtà si seguono le autorità.

## 5. Il bilanciamento è parte del design

Il gioco insegna qualcosa solo se **nessuna strategia banale vince**: «evacua tutti
subito», «tutte le squadre sull'innesco», «non fare niente». Si misura con
`sim-game-balance`, prima di decidere soglie e meccaniche.

## 5b. Provare una meccanica nuova prima di costruirla

- **Una domanda sola**, se possibile sì/no («il giocatore capisce da solo che la
  previsione si chiede con la pausa?»). Due domande, due prove.
- Si scrive prima: domanda, azione centrale, se la prova si butta o si tiene, tempo
  massimo, **«si tiene se…»** e **«si scarta se…»** con un segnale osservabile. Senza un
  criterio di scarto non si scarta niente.
- Grafica grezza, nessun menu né salvataggio: solo ciò da cui dipende la domanda. I valori
  misurati si mostrano sullo schermo o nel runner headless.
- Segnale tipico: una persona che non conosce il gioco compie l'azione da sola entro
  circa 30 secondi e la ripete senza che glielo si chieda (`agent-playtest`). Se la
  meccanica funziona solo dopo una spiegazione, va ripensata, non spiegata meglio.
- La prova sta fuori dal gioco (un esempio, un flag spento, una cartella a parte). Se si
  tiene, si riscrive per bene nel nucleo; il codice della prova non diventa il prodotto.
  Il resto segue il metodo per fasi (`phased-multiagent-workflow`): accesa nel gioco solo
  dopo la conferma dell'utente.

## 6. Esempio: Isola di Roccavento

Incendio boschivo su un'isola immaginaria. RTS con pausa: ×120 o fermo. Le leve sono
5 squadre AIB (autobotte e 3 persone) su una griglia esagonale e l'evacuazione di un
borgo alla volta, al massimo 6 persone al minuto. La previsione è fatta di isocrone a
+1 e +3 h, al 50 % di 16 simulazioni. Il finale ha stelle Persone, Case ed Ettari
rispetto al «senza ordini». Documenti: `docs/00-SPECIFICHE.md`, `docs/01-GIOCO.md`,
`docs/04-VALUTAZIONE.md`.

Errori che sono costati iterazioni:

- il gioco a turni con «Decidi»;
- il limite di 3 pause;
- il punteggio con penalità per i cambi di piano;
- le aree di raccolta fuori dai borghi, bocciate perché «irrealistiche per la protezione
  civile»;
- l'evacuazione a piedi;
- il giocatore nel ruolo del sindaco, che è già un personaggio.

---

Il §5b adatta `prototype-fast` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0). Nessun testo copiato.
