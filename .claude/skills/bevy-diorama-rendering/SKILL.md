---
name: bevy-diorama-rendering
description: Rendering 3D in stile diorama per giochi di rischio naturale in Bevy (terreno, coste e mare, vegetazione che brucia, fuoco, fumo, acqua dei mezzi, segni di gioco sul terreno, camera e regia demo, livelli di qualità e FPS). Usare quando si lavora su shader WGSL, materiali, mesh, particelle, overlay, camera o prestazioni grafiche, o quando un effetto «non si vede», è instabile o costa troppo.
---

# Rendering del diorama (Bevy)

Il mondo 3D serve a far sentire il rischio. Non deve competere con gli elementi di gioco,
e deve girare anche su un computer meno potente e nel browser.

## 1. Principio: shader prima della geometria

- Terreno, coste, strade, erba lontana, aree bruciate e segni di gioco rendono meglio e
  costano meno **nel pixel shader del terreno** che come mesh sovrapposte. Si evitano
  segmenti staccati, z-fighting, bordi neri e linee che fluttuano.
- La geometria vera si tiene per ciò che sporge: alberi, case, mezzi, fiamme. Il dettaglio
  vicino alla camera (fili d'erba) va nel vertex shader, con un ripiego nel pixel shader per
  il contrasto a distanza.
- I segni di gioco (strade evidenziate o chiuse, aree raggiungibili, aree d'intervento,
  previsione, percorso di evacuazione con tratteggio animato) entrano nello shader come
  maschere o texture di dati aggiornate dal nucleo. La grafica legge e basta (vedi
  `hazard-sim-integration`).

## 2. Trappole verificate

- **Winding:** una striscia stesa sul terreno con il winding sbagliato è invisibile.
  Avvolgerla `a, a+2, a+1` e controllare in che verso corrono le righe del reticolo.
- **Linee drappeggiate:** vanno ricampionate ogni circa 5 m, altrimenti tagliano il terreno.
- **Segni:** vanno alzati sopra la chioma, di +20 m o più.
- **Una mesh senza attributi** (vuota) manda in panico la pipeline di prepass all'avvio. Non
  lasciare mesh segnaposto vuote.
- **Nebbia:** un `Material` personalizzato non la riceve da solo. Va aggiunta nello shader,
  oppure va accettato che il materiale (overlay) ne resti fuori.
- **Mare** con superficie spostata nel vertex shader:
  - va spento il depth prepass, perché la profondità piatta del prepass crea bande;
  - la normale va calcolata dalla pendenza dell'onda, non dalle facce del reticolo.
- **Riavvio:** vanno ripuliti gli stati che si ricordano qualcosa (case annerite, particelle,
  chiavi dei segni).
- **Instabilità:** l'erba e i dettagli fini che sfarfallano al minimo movimento della camera
  vanno stabilizzati, ancorandoli al mondo e non allo schermo e sfumandoli con la distanza.
  Il dettaglio va deciso sulla profondità del pixel, non su un disco attorno al centro
  della vista.
- **Trasparenze delle particelle:** un bordo scuro attorno alla particella di solito viene
  dal blending (alfa non premoltiplicato) o dal campionamento della texture. Le particelle
  tonde o ovali si disegnano nel pixel shader, senza texture. Prima si conta, poi si riduce:
  spesso ne basta il 25 %.
- **Shader WGSL, controlli generici:** validare il sorgente con `naga` prima di avviare
  il gioco; normalizzare normali e direzioni nel pixel shader (l'interpolazione le
  accorcia); preferire `mix`/`step`/`smoothstep`/`clamp` ai rami che dipendono dal pixel;
  mai `smoothstep` con i bordi invertiti (`1.0 - smoothstep(a, b, x)`); `discard` solo
  come uscita anticipata, perché spegne l'early-Z e costa sulle GPU dei telefoni; tempo e
  coordinate grandi in precisione piena, se no sfarfallano su WebGL2.

## 3. Vegetazione e fuoco

- La vegetazione disegnata si legge dal **combustibile del modello**, così ciò che si vede
  corrisponde a ciò che brucia. Va diradata attorno alle strade, e non va disegnata dove è
  già bruciato.
- Alberi e arbusti si modellano in **Blender** con script ripetibili: varietà per tipo,
  pochi poligoni, corteccia e chioma nel pixel shader.
- **Bruciatura:** due modelli sovrapponibili, verde e bruciato. Da vicino uno shader
  passa progressivamente dall'uno all'altro, senza un modello grigio intermedio; da lontano
  si usa un modello bruciato semplificato.
- **Aree bruciate:** il bordo tra bruciato e non bruciato va frattale, con variazioni di
  marrone e grigio e un materiale poco «plastico». Se dentro un'area bruciata restano
  macchie verdi, va verificato da dove vengono (celle non bruciate del modello o un errore
  di campionamento) prima di coprirle.
- **Fiamme:** lingue animate (shader), non sprite che vanno su e giù. Ogni tanto si passa
  in chioma dove ci sono alberi alti. Braci piccole e poche.
- **Acqua dei mezzi:** un arco solo che ruota tra le direzioni, senza splash. La logica
  interna non cambia.
- **Spegnimento visibile:** le linee tagliate tolgono le piante lungo una linea definita, non
  a caso.

## 4. Camera e regia

- **Controlli:** trascinare per spostare, tasto destro o Maiusc per ruotare, rotella o
  pizzico per lo zoom, Home per la vista iniziale. La camera non entra nel terreno.
- **Regia della demo** (schermata iniziale): segue l'azione sul fuoco e alterna ogni tanto
  inquadrature sceniche dei punti d'interesse. Angoli bassi che esaltano l'effetto
  diorama, transizioni in dissolvenza nei cambi bruschi, movimenti lenti e poco frequenti.
  La quota si filtra perché non segua le micro variazioni del terreno.
- **Movimento indipendente dagli FPS:** smorzamento esponenziale (`1 - exp(-k·dt)`), non un
  `lerp` a fattore fisso; la camera si aggiorna dopo che i bersagli si sono mossi, se no
  trema di un frame. Sui salti voluti (clic su un avviso, riavvio) si azzera lo
  smorzamento o si usa una transizione breve.
- **Inclinazione limitata** in entrambi i versi, e sensibilità del trascinamento
  indipendente dalla dimensione della finestra. Una eventuale scossa si somma per ultima,
  sopra l'inseguimento, e decade (rumore liscio, non valori casuali a ogni frame).
- **Pausa tattica:** si desaturano gli elementi non di gioco, di poco.

## 5. Qualità e prestazioni

- **Livelli** (alta, media, bassa, più i profili web) in un solo modulo, scelti da
  variabile d'ambiente, menu o dalla pagina web. Ognuno elenca che cosa spegne: ombre,
  profondità di campo, bloom, MSAA, densità della vegetazione, passo del terreno, tetti di
  fiamme, fumo e braci.
- **Si misura prima di ottimizzare:** un indicatore di FPS filtrato nel tempo e una misura
  per stato con lo stesso harness degli screenshot (`game-snapshot-harness`).
  Obiettivo indicativo: circa 120 FPS su un Apple M4 Pro. I colli di bottiglia tipici
  sono il fumo e le trasparenze, le ombre, troppe istanze di vegetazione e i ricalcoli a
  ogni frame.
- Le mesh statiche (terreno, mare) rilasciano la copia CPU dopo il caricamento.
- Ogni ottimizzazione si verifica con gli stessi screenshot prima e dopo: l'immagine deve
  restare la stessa.

## 6. Esempio: Isola di Roccavento

Bevy 0.20 (codice di gioco e prestazioni: `bevy-game-code`). Shader in `crates/game/src/shaders/*.wgsl` (`flat` per i segni,
`forecast_zone`, `reach_zone`, `route_line`, `tape`, `water`, `flame`, `fire_ground`,
`near_cover`, `sky`). Livelli in `crates/game/src/quality.rs`. Modelli Blender in
`scripts/build_*_models.py`. Trappole aggiornate nella sezione «Trappole note» di
`CLAUDE.md`.

---

Le note su camera e shader WGSL dei §2 e §4 adattano `camera-systems` e
`shader-programming` di
[awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills)
(© 2026 Abhishek Barali e contributori, Apache-2.0). Nessun testo copiato.
