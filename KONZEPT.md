# UwUSweep — Konzept

> Programme deinstallieren und hinterher aufkehren, was sie liegen lassen.
> Ohne Konto, ohne Netz, mit Nyu.

|                 |                                                                       |
| --------------- | --------------------------------------------------------------------- |
| **Stand**       | 2026-11-07 · 0.1.0-beta.1                                             |
| **Basis**       | Tauri 2 + React wie die anderen UwU-Apps, Logik in Rust               |
| **Bundle-ID**   | `app.uwusweep.desktop`                                                |
| **Repo**        | [MinifyX/UwUSweep](https://github.com/MinifyX/UwUSweep) · GPL-3.0     |
| **Maskottchen** | Nyu, diesmal als Kehrschaufel, mit Besen                              |
| **Plattformen** | Windows (x64, ARM), macOS (Apple-Chip, Intel), Linux (x64, ARM), Arch |

---

## 1. Ziel

Ein Deinstaller, der nach dem eigentlichen Deinstallieren nachsieht, was
liegen geblieben ist — Ordner, Einstellungen, Registry-Schlüssel — und das
vorsichtig wegräumt: erst in die Quarantäne, endgültig erst nach 30 Tagen.
Eigener Code von Grund auf, im Stil der UwUSuite, auf Deutsch und Englisch.

### Nicht-Ziele

- Kein „PC-Optimierer“: keine Registry-Reinigung ohne App-Bezug, keine
  Cache-Putzaktionen, keine Leistungsversprechen.
- Kein endgültiges Löschen ohne Umweg über die Quarantäne.
- Keine Telemetrie, keine Updates aus dem Netz, keine Konten.

## 2. Was es kann (0.1.0-beta.1)

1. **App-Liste** — Windows: Uninstall-Schlüssel in HKLM/HKCU/WOW6432Node
   (inkl. MSI), Store-Apps, winget-IDs; macOS: `.app` in `/Applications` und
   `~/Applications`; Linux: Pakete mit `.desktop`-Datei (dpkg, pacman, rpm),
   Flatpak, Snap.
2. **Deinstallieren** — eigener Deinstaller bzw. Paketmanager, UwUSweep wartet
   darauf. Windows optional mit Wiederherstellungspunkt vorher. macOS hat
   keinen Deinstaller: Das Bundle selbst taucht als sicherer Rest auf.
3. **Reste-Suche** — Suchwörter aus Name, Hersteller, Ordner und Hinweisen.
   _Sicher_: nur exakte Treffer (vorausgewählt). _Gründlich_: auch
   enthaltene Treffer (nicht vorausgewählt). Geschützte Pfade (Systemordner,
   Home, die Suchwurzeln selbst) nie.
4. **Quarantäne** — pro Vorgang ein Ordner mit `manifest.json`; Dateien
   verschoben, Registry als JSON exportiert und dann gelöscht. Rückgängig per
   Klick, nach N Tagen (Standard 30) automatisch endgültig weg.
5. **Mitschreiben** — Schnappschuss vor und nach einer Installation (Dateien
   in den Suchwurzeln, Registry, App-Liste), der Unterschied wird gespeichert.
   Beim Deinstallieren der App sind diese Einträge sichere Reste.
6. **Autostart** — Einträge auflisten, ein- und ausschalten.

## 3. Architektur

```
┌─ WebView (React, @uwusuite/design) ─────────────┐
│  App-Liste, Reste, Quarantäne, Mitschriften,    │
│  Autostart, Einstellungen                       │
└───────────────┬─────────────────────────────────┘
                │ Tauri-Befehle (JSON)
┌───────────────┴─────────────────────────────────┐
│  Rust: uwusweep-core                            │
│  platform/{windows,macos,linux}  terms  scan    │
│  quarantine  snapshot  helper                   │
└───────────────┬─────────────────────────────────┘
                │ nur bei Bedarf erhöht:
                │ uwusweep --uwusweep-helper <job.json> <sha256>
                ▼
        UAC · macOS-Adminabfrage · pkexec
```

**Admin-Rechte nur über den Helfer.** Die App selbst läuft ohne Rechte. Braucht
ein Schritt mehr, schreibt sie einen Auftrag als JSON und startet dieselbe
Binary erhöht mit Pfad und SHA-256 des Auftrags auf der Kommandozeile. Der
Helfer prüft den Hash (die Kommandozeile kann kein anderer Prozess mehr
ändern), führt aus und schreibt `<job>.result.json`.

## 4. Sicherheit und Datenschutz

- Kein Netz: UwUSweep spricht mit keinem Server.
- Gespeichert werden nur Einstellungen, die Quarantäne und die Mitschriften im
  App-Datenordner.
- Der Helfer traut dem Auftrag nicht blind: Quarantäne-Ziele müssen unter dem
  Quarantäne-Ordner liegen, geschützte Pfade werden nie angefasst, Befehle
  laufen als argv, nie über eine Shell. Ausnahme ist der Windows-
  UninstallString, der als Kommandozeile aus der Registry kommt.
