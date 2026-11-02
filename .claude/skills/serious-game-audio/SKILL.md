---
name: serious-game-audio
description: Audio di un serious game di rischio naturale che deve restare giocabile senza suono - ambiente generato o registrato che segue camera e simulazione, segnali brevi per eventi e ordini, gruppi di volume, cursori percepiti, ducking, variazione, limiti di polifonia, avvio nel browser e accessibilità. Usare quando si aggiungono o si mixano suoni, cursori del volume, musica, allarmi sonori, o quando l'audio «non parte» sul web.
---

# Audio di un gioco di rischio

L'audio fa sentire il luogo e il pericolo, ma **il gioco deve restare uguale da muto**:
in molte occasioni (scuola, evento, telefono in tram) si gioca senza suono. Ogni segnale
sonoro ha quindi un gemello visivo, e nessuna regola dipende dall'audio.

## 1. Che cosa suona

- **Ambiente** continuo che segue lo stato: vento (velocità del vento simulato), fuoco
  (vicinanza del fronte alla camera), mare, animali che tacciono vicino al fuoco. Si
  calcola dalla simulazione e dalla camera, non decide niente.
- **Segnali** brevi per ciò che lo schermo già mostra: pausa e ripresa, ordine dato,
  ordine rifiutato, messaggio in chat, casa colpita, squadra in pericolo, evacuazione
  completata, finale.
- **Livelli di importanza** (piccolo, medio, grande): ogni evento ne ha uno, e il livello
  fissa volume, durata e se interrompe gli altri. Un clic non suona come un borgo perso.
- Musica: facoltativa. Se c'è, a strati che entrano con l'intensità (fronte vicino,
  case colpite) invece di un brano solo in loop, che stanca.

## 2. Mix

- **Gruppi, non volumi per suono**: ambiente, segnali, interfaccia (e voce o musica se
  ci sono), sotto un generale. I cursori delle impostazioni comandano i gruppi.
- **Cursori percepiti**: il volume si sente in modo logaritmico. Il cursore 0–1 si eleva
  al quadrato (o si converte in dB) prima di diventare guadagno; a 0 il gruppo è muto
  davvero. Un cursore lineare cambia quasi tutto nell'ultimo tratto.
- **Ducking**: quando arriva un segnale importante l'ambiente scende di qualche dB e
  risale in 0,3–0,5 s. Il ducking è un guadagno a parte, che **non sovrascrive** il
  cursore scelto dal giocatore. Un rilascio troppo rapido «pompa».
- **Margine**: picchi sotto −1 dBTP; sonorità media attorno a −18 LUFS per telefono e
  portatile, −24 LUFS per altoparlanti e TV. I −14 LUFS della musica in streaming non
  sono un obiettivo per un gioco. Un limitatore sul generale fa da rete, non da mixer.
- **Variazione**: per i suoni ripetuti, piccole variazioni casuali di altezza (±1
  semitono) e volume (±2 dB), più campioni a rotazione senza ripetere l'ultimo.
- **Polifonia limitata**: al massimo poche copie dello stesso segnale insieme (es. 4);
  le istanze finite si liberano (in Bevy `PlaybackSettings::DESPAWN`).

## 3. Web e piattaforme

- Il browser tiene sospeso l'`AudioContext` fino al primo gesto dell'utente: si riprende
  a ogni gesto, e il gioco non deve aspettare l'audio per partire.
- Musica e ambienti lunghi in streaming o generati nel codice; i segnali brevi in memoria,
  già decodificati. Pochi MB in tutto: sul web l'audio pesa sul download.
- Le preferenze (muto, volumi) si salvano come quelle della grafica (file o storage del
  browser).
- Con le prove automatiche (screenshot, playtest headless) il volume è zero.

## 4. Verifica

- Si ascolta su cuffie e su altoparlanti di un portatile: equilibrio, ducking udibile ma
  non pompato, nessun segnale che copre gli altri.
- Si gioca una partita intera **da muti** e si controlla che ogni segnale abbia il suo
  equivalente visivo.
- Si misura la memoria e il tempo CPU dell'audio generato (`bevy-game-code` §4).

## 5. Esempio: Isola di Roccavento

`crates/game/src/sound.rs` (preferenze in `~/.rocca_ventosa/audio.txt` o nello storage
del browser, ambiente generato nel crate `sfx`, cicale e gabbiani registrati in
`data/sounds`, segnali di interfaccia di Kenney), ripresa dell'`AudioContext` in
`web/index.html`. Il volume a zero sotto `KIOSK_SNAP` è in `main.rs`.

---

Fonte: i §1–2 adattano e riassumono `audio-design` e in parte `game-feel` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0); i valori di sonorità vi sono citati da
Sony ASWG-R001. Nessun testo copiato; le parti sul gioco muto, sul web e su Roccavento sono
del progetto.
