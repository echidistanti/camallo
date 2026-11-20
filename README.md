# Camallo

Scarica i video da un URL o da un browser incorporato che li rileva da solo. Windows, scritto in Rust (egui, wry/WebView2), con yt-dlp e ffmpeg come motore (scaricati e aggiornati dall'app).

- Flusso: incolli il link di un video o di una playlist, yt-dlp ne legge le informazioni, scegli tipo e qualità per ogni singolo video (con miniatura, durata e dimensione stimata) e premi Download. Una playlist diventa un gruppo di schede, una per video.
- Se yt-dlp non riesce a leggere l'indirizzo, la scheda propone il browser incorporato: apri la pagina, riproduci il video e compare il riquadro "Video detected" con il tasto Download.
- Rilevamento a livello di rete (risposte HTTP di WebView2) più uno script iniettato nella pagina.
- MP4/MKV/WebM, solo audio, coda con download in parallelo, cronologia, avvio automatico opzionale (Impostazioni).
- Finestra senza barra di sistema: barra del titolo nera disegnata dall'app, accento viola.

## Installer

Ogni release pubblicata riceve in automatico `camallo-<versione>-windows-x64-setup.exe`: wizard Inno Setup (inglese e italiano), installazione per utente senza permessi di amministratore in `%LOCALAPPDATA%\Programs\Camallo`, voce in Installazioni applicazioni e disinstallatore che chiede se togliere anche impostazioni e cronologia. Lo costruisce `.github/workflows/installer.yml` a partire dal `.7z` già allegato alla release; per rifarlo a mano: Actions, Installer, Run workflow, con il tag. Lo script è `installer/camallo.iss`.

## Compilare

```
cargo test
cargo test -- --ignored real   # richiede CAMALLO_YTDLP, CAMALLO_FFDIR, CAMALLO_BASE (server locale con clip.mp4, hls/master.m3u8 e playlist.html con due <video>)
node tests/detect_test.js
cargo build --release --target x86_64-pc-windows-gnu   # da Linux, serve mingw-w64
```

Su Linux l'app si avvia ma il browser incorporato no. Per provare la finestra, `CAMALLO_FAKE_FEED=file.jsonl` rimanda al rilevatore messaggi registrati.
