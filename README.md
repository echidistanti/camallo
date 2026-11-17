# Camallo

Scarica i video da un URL o da un browser incorporato che li rileva da solo. Windows, scritto in Rust (egui, wry/WebView2), con yt-dlp e ffmpeg come motore (scaricati e aggiornati dall'app).

- Rilevamento a livello di rete (risposte HTTP di WebView2) più uno script iniettato nella pagina.
- Riquadro "Video detected" con tasto Download, scelta della qualità, MP4/MKV/WebM, solo audio, coda con download in parallelo, cronologia.

## Compilare

```
cargo test
cargo test -- --ignored real   # richiede CAMALLO_YTDLP, CAMALLO_FFDIR, CAMALLO_BASE (server locale con un video)
node tests/detect_test.js
cargo build --release --target x86_64-pc-windows-gnu   # da Linux, serve mingw-w64
```

Su Linux l'app si avvia ma il browser incorporato no. Per provare la finestra, `CAMALLO_FAKE_FEED=file.jsonl` rimanda al rilevatore messaggi registrati.
