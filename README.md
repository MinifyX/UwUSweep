<p align="center">
  <img src="brand/uwusweep-app-icon.svg" width="112" alt="UwUSweep-Logo" />
</p>

<h1 align="center">UwUSweep</h1>

<p align="center">
  Programme deinstallieren und hinterher aufkehren, was sie liegen lassen. (◕‿◕✿)<br/>
  Windows · macOS · Linux, erste Beta
</p>

<p align="center">
  <a href="https://github.com/MinifyX/UwUSweep/releases"><b>Download für Windows, macOS und Linux</b></a>
</p>

---

## Was es kann

- **App-Liste:** alles, was installiert ist – unter Windows aus der Registry
  (auch MSI), Store-Apps und winget; unter macOS die `.app`s in
  `/Applications` und `~/Applications`; unter Linux Pakete mit Menüeintrag
  (dpkg, pacman, rpm), Flatpak und Snap.
- **Deinstallieren:** startet den Deinstaller der App bzw. den Paketmanager und
  wartet auf ihn. Unter Windows auf Wunsch vorher ein Wiederherstellungspunkt.
- **Reste suchen:** Ordner, Dateien und (unter Windows) Registry-Schlüssel, die
  zur App gehören. Stufe _Sicher_ nimmt nur eindeutige Treffer, _Gründlich_
  auch ähnliche. Eindeutige Reste sind vorausgewählt, der Rest nicht.
- **Quarantäne statt Löschen:** Reste werden verschoben, Registry-Schlüssel
  vorher exportiert. Ein Klick holt alles zurück; nach 30 Tagen (einstellbar)
  wird endgültig gelöscht.
- **Installationen mitschreiben:** Schnappschuss vor und nach einem Setup. Was
  dazugekommen ist, findet UwUSweep beim Deinstallieren sicher wieder.
- **Autostart:** sehen, was beim Anmelden mitstartet, und es ein- oder
  ausschalten.

Admin-Rechte fragt UwUSweep nur an, wenn ein Schritt sie wirklich braucht
(UAC, die macOS-Passwortabfrage bzw. pkexec). Keine Telemetrie, kein Konto,
kein Netz.

## Installation

Unter [Releases](https://github.com/MinifyX/UwUSweep/releases) liegt für jedes
System ein Setup:

| System                    | Datei                                         |
| ------------------------- | --------------------------------------------- |
| Windows (x64, ARM)        | `UwUSweep-windows-x64-setup.exe` / `-arm64-…` |
| macOS (Apple-Chip, Intel) | `UwUSweep-macos-universal.dmg`                |
| Debian, Ubuntu            | `UwUSweep-linux-x64.deb` / `-arm64.deb`       |
| Fedora, openSUSE          | `UwUSweep-linux-x64.rpm` / `-arm64.rpm`       |
| Arch                      | `PKGBUILD` im Release                         |
| andere Linux              | `UwUSweep-linux-x64-portable.tar.gz` / ARM    |

UwUSweep ist nicht bei Apple notarisiert: Meldet macOS, es könne die App nicht
prüfen, unter **Systemeinstellungen → Datenschutz & Sicherheit → Trotzdem
öffnen** freigeben.
Unter Linux braucht das Deinstallieren von Systempaketen `pkexec` (polkit).

## Entwicklung

Voraussetzungen: Node.js 22 und pnpm 10 (`corepack enable`), Rust stable,
dazu die [Tauri-Voraussetzungen](https://tauri.app/start/prerequisites/)
für das eigene System.

```bash
pnpm install
pnpm tauri dev
```

Prüfen wie in CI:

```bash
pnpm lint && pnpm typecheck && pnpm test
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Setup bauen (landet in `target/installers/`): `pnpm build:setup`.
Ein Release ist ein Tag; CI baut alle Installer und legt den Release-Entwurf mit
den Notizen aus `release-notes/` an, siehe
[release-notes/README.md](release-notes/README.md).

| Pfad                   | Inhalt                                                          |
| ---------------------- | --------------------------------------------------------------- |
| `apps/desktop`         | Die Tauri-2-App (React-Oberfläche + Rust-Hülle)                 |
| `apps/setup`           | Installer und Deinstaller für alle drei Systeme                 |
| `crates/uwusweep-core` | App-Listen, Reste-Suche, Quarantäne, Mitschreiben, Admin-Helfer |
| `brand/`               | Nyu als Kehrschaufel: Icon, Symbol, Taskleisten-Icons           |
| `scripts/`             | Setup, Icons, Release-Text, AUR-Paket                           |

Mehr: [KONZEPT.md](KONZEPT.md) · [Roadmap](docs/roadmap.md) ·
[CHANGELOG](CHANGELOG.md)

## Lizenz

[GNU GPL v3.0](LICENSE).
